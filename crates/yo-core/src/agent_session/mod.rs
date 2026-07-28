use std::{
    collections::{HashSet, VecDeque},
    error::Error,
    fmt,
    num::NonZeroU64,
    sync::{
        Arc, Condvar, Mutex, TryLockError,
        atomic::{AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    ActivityKind, ActivityRequestRef, ActivityResponse, ActivityUpdate, AgentBackend, AgentCommand,
    AgentEvent, AgentRuntime, ApprovalDecision, BackendFailure, BackendStopHandle, RuntimeError,
    RuntimePoll, SessionId, TurnId, TurnRef, UserInput,
};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const WORKER_GRACEFUL_SHUTDOWN: Duration = Duration::from_millis(50);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const COMMAND_CAPACITY: usize = 32;
const URGENT_COMMAND_CAPACITY: usize = 8;
const EVENT_CAPACITY: usize = 256;
const WORKER_IDLE: u8 = 0;
const WORKER_EXECUTING: u8 = 1;
const WORKER_STOPPING: u8 = 2;

/// A frontend intent directed at an agent Session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentIntent {
    /// Starts a Turn or steers the active Turn.
    Submit(String),
    /// Requests interruption of the active Turn.
    Interrupt,
    /// Answers one correlated approval request.
    RespondToApproval {
        /// The outstanding request being answered.
        request: ActivityRequestRef,
        /// The user's approval decision.
        decision: ApprovalDecision,
    },
    /// Answers one correlated agent-requested input.
    RespondToUserInput {
        /// The outstanding request being answered.
        request: ActivityRequestRef,
        /// The user's response text.
        input: String,
    },
}

/// Immediate result of placing an intent on an agent Session.
#[derive(Debug, Eq, PartialEq)]
pub enum CommandAdmission {
    /// The Session retained the command for delivery.
    Accepted,
    /// The Session is busy; the frontend retains and retries this operation.
    Backpressured(PendingCommand),
}

/// An opaque, single-use operation retained across nonblocking backpressure.
#[derive(Debug, Eq, PartialEq)]
pub struct PendingCommand(AgentCommand);

impl PendingCommand {
    fn from_command(command: AgentCommand) -> Self {
        Self(command)
    }

    fn into_command(self) -> AgentCommand {
        self.0
    }
}

#[derive(Default)]
struct SessionState {
    active_turn: Option<TurnRef>,
    turn_started: bool,
    interrupted_turns: HashSet<TurnRef>,
    outstanding_requests: HashSet<ActivityRequestRef>,
}

/// Nonblocking frontend-independent connection to one worker-owned agent runtime.
pub struct AgentSession {
    commands: SyncSender<AgentCommand>,
    urgent_commands: SyncSender<AgentCommand>,
    events: Option<Receiver<WorkerEvent>>,
    finished: Receiver<()>,
    stop: BackendStopHandle,
    failure: Arc<Mutex<Option<AgentSessionError>>>,
    lifecycle: Arc<AtomicU8>,
    session_id: SessionId,
    state: Arc<Mutex<SessionState>>,
    active_turn_id: Arc<AtomicU64>,
    next_turn_id: u64,
    #[cfg(test)]
    processed: Arc<(Mutex<u64>, Condvar)>,
    worker: Option<JoinHandle<WorkerExit>>,
}

impl AgentSession {
    /// Starts a Session and waits for its initial backend handshake.
    pub fn start<B>(backend: B) -> Result<Self, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        let mut never_cancelled = || false;
        Ok(Self::start_inner(backend, &mut never_cancelled)?
            .expect("a callback that always returns false cannot cancel startup"))
    }

    /// Starts a Session while allowing its frontend host to cancel the handshake.
    ///
    /// Returns `Ok(None)` after a requested cancellation and complete backend
    /// cleanup, before a frontend takes ownership of the Session.
    pub fn start_cancellable<B>(
        backend: B,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        Self::start_inner(backend, &mut is_cancelled)
    }

    fn start_inner<B>(
        backend: B,
        is_cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        let stop = backend.stop_handle();
        let session_id = SessionId::new(NonZeroU64::MIN);
        let (command_tx, command_rx) = mpsc::sync_channel(COMMAND_CAPACITY);
        let (urgent_tx, urgent_rx) = mpsc::sync_channel(URGENT_COMMAND_CAPACITY);
        let (event_tx, event_rx) = mpsc::sync_channel(EVENT_CAPACITY);
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let (finished_tx, finished_rx) = mpsc::sync_channel(1);
        let (backend_tx, backend_rx) = mpsc::sync_channel(1);
        let failure = Arc::new(Mutex::new(None));
        let worker_failure = Arc::clone(&failure);
        let state = Arc::new(Mutex::new(SessionState::default()));
        let worker_state = Arc::clone(&state);
        let active_turn_id = Arc::new(AtomicU64::new(0));
        let worker_active_turn_id = Arc::clone(&active_turn_id);
        let processed = Arc::new((Mutex::new(0), Condvar::new()));
        let worker_processed = Arc::clone(&processed);
        let lifecycle = Arc::new(AtomicU8::new(WORKER_IDLE));
        let worker_lifecycle = Arc::clone(&lifecycle);
        let worker = match thread::Builder::new()
            .name("yo-agent-runtime".to_owned())
            .spawn(move || {
                let Ok(backend) = backend_rx.recv() else {
                    let _ = finished_tx.send(());
                    return WorkerExit::success();
                };
                let mut worker =
                    AgentWorker::new(backend, session_id, worker_state, worker_active_turn_id);
                let outcome = match worker.initialize() {
                    Ok(events) => {
                        if startup_tx.send(Ok(())).is_err() {
                            WorkerExit::from_cleanup(worker.runtime.shutdown())
                        } else {
                            let mut lane = EventLane::new(event_tx, worker_failure);
                            if !lane.send_events(events) {
                                WorkerExit::from_cleanup(worker.runtime.shutdown())
                            } else {
                                worker.run(
                                    command_rx,
                                    urgent_rx,
                                    &mut lane,
                                    &worker_processed,
                                    &worker_lifecycle,
                                )
                            }
                        }
                    },
                    Err(error) => {
                        let _ = startup_tx.send(Err(error));
                        WorkerExit::success()
                    },
                };
                let _ = finished_tx.send(());
                outcome
            }) {
            Ok(worker) => worker,
            Err(error) => {
                stop.request_stop();
                let mut backend = backend;
                return match backend.shutdown() {
                    Ok(()) => Err(AgentSessionError::WorkerUnavailable(error.to_string())),
                    Err(cleanup) => Err(AgentSessionError::Multiple {
                        primary: Box::new(AgentSessionError::WorkerUnavailable(error.to_string())),
                        additional: Box::new(AgentSessionError::BackendCleanup(cleanup)),
                    }),
                };
            },
        };
        if let Err(error) = backend_tx.send(backend) {
            stop.request_stop();
            let mut backend = error.0;
            let cleanup = backend
                .shutdown()
                .err()
                .map(AgentSessionError::BackendCleanup);
            let _ = join_worker(worker)?;
            return Err(cleanup.unwrap_or_else(|| {
                AgentSessionError::WorkerUnavailable(
                    "the runtime worker closed before receiving its backend".to_owned(),
                )
            }));
        }

        let mut terminated = false;
        let mut termination_started = None;
        let startup = loop {
            match startup_rx.recv_timeout(WORKER_POLL_INTERVAL) {
                Ok(startup) => break startup,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = join_worker(worker)?;
                    return Err(AgentSessionError::WorkerUnavailable(
                        "the runtime worker closed during startup".to_owned(),
                    ));
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if termination_started.is_some_and(|started: Instant| {
                        started.elapsed() >= WORKER_SHUTDOWN_TIMEOUT
                    }) {
                        return Err(AgentSessionError::WorkerShutdownTimedOut);
                    }
                    if is_cancelled() {
                        terminated = true;
                        termination_started.get_or_insert_with(Instant::now);
                        stop.request_stop();
                    }
                },
            }
        };

        match startup {
            Ok(()) => {
                let mut app = Self {
                    commands: command_tx,
                    urgent_commands: urgent_tx,
                    events: Some(event_rx),
                    finished: finished_rx,
                    stop,
                    failure,
                    lifecycle,
                    session_id,
                    state,
                    active_turn_id,
                    next_turn_id: 1,
                    #[cfg(test)]
                    processed,
                    worker: Some(worker),
                };
                if terminated {
                    app.shutdown()?;
                    Ok(None)
                } else {
                    Ok(Some(app))
                }
            },
            Err(error) => {
                let _ = join_worker(worker)?;
                if terminated {
                    match error {
                        AgentSessionError::StartAndCleanup { cleanup, .. } => {
                            Err(AgentSessionError::Runtime(*cleanup))
                        },
                        _ => Ok(None),
                    }
                } else {
                    Err(error)
                }
            },
        }
    }

    /// Stops the worker, cleans up the backend, and returns terminal events.
    pub fn shutdown(&mut self) -> Result<Vec<AgentEvent>, AgentSessionError> {
        let Some(worker) = self.worker.take() else {
            return Ok(Vec::new());
        };
        self.lifecycle.store(WORKER_STOPPING, Ordering::Release);
        let finished_gracefully = match self.finished.recv_timeout(WORKER_GRACEFUL_SHUTDOWN) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => true,
            Err(mpsc::RecvTimeoutError::Timeout) => false,
        };
        if !finished_gracefully {
            self.stop.request_stop();
        }

        let mut failures = Vec::new();
        if let Some(events) = self.events.take() {
            drop(events);
        }

        if !finished_gracefully && self.finished.recv_timeout(WORKER_SHUTDOWN_TIMEOUT).is_err() {
            if let Some(failure) = self.take_failure() {
                failures.push(failure);
            }
            failures.push(AgentSessionError::WorkerShutdownTimedOut);
            return Err(combine_failures(failures).expect("the shutdown timeout added one failure"));
        }
        match join_worker(worker) {
            Ok(exit) => {
                if let Some(failure) = self.take_failure() {
                    failures.push(failure);
                }
                if let Some(failure) = exit.failure {
                    failures.push(failure);
                }
                if let Some(error) = combine_failures(failures) {
                    Err(error)
                } else {
                    Ok(exit.terminal_events)
                }
            },
            Err(error) => {
                if let Some(failure) = self.take_failure() {
                    failures.push(failure);
                }
                failures.push(error);
                Err(combine_failures(failures).expect("join added one failure"))
            },
        }
    }

    fn take_failure(&self) -> Option<AgentSessionError> {
        self.failure.lock().ok()?.take()
    }

    fn resolve_action(
        &mut self,
        action: AgentIntent,
        state: &mut SessionState,
    ) -> Result<AgentCommand, AgentSessionError> {
        match action {
            AgentIntent::Submit(text) => {
                let input = UserInput::new(text);
                if let Some(turn) = state.active_turn {
                    if state.interrupted_turns.contains(&turn) {
                        return Err(AgentSessionError::TurnInterruptPending);
                    }
                    Ok(AgentCommand::SteerTurn { turn, input })
                } else {
                    let turn = self.next_turn()?;
                    state.active_turn = Some(turn);
                    state.turn_started = false;
                    self.active_turn_id
                        .store(turn.turn_id().get().get(), Ordering::Release);
                    Ok(AgentCommand::StartTurn { turn, input })
                }
            },
            AgentIntent::Interrupt => Ok(AgentCommand::InterruptTurn {
                turn: state.active_turn.ok_or(AgentSessionError::NoActiveTurn)?,
            }),
            AgentIntent::RespondToApproval { request, decision } => {
                if !state.outstanding_requests.contains(&request) {
                    return Err(AgentSessionError::NoOutstandingRequest);
                }
                Ok(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::Approval(decision),
                })
            },
            AgentIntent::RespondToUserInput { request, input } => {
                if !state.outstanding_requests.contains(&request) {
                    return Err(AgentSessionError::NoOutstandingRequest);
                }
                Ok(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::UserInput(UserInput::new(input)),
                })
            },
        }
    }

    fn queue_command(
        &mut self,
        command: AgentCommand,
        state: &mut SessionState,
    ) -> Result<CommandAdmission, AgentSessionError> {
        match &command {
            AgentCommand::StartTurn { turn, .. } => match state.active_turn {
                None => {
                    state.active_turn = Some(*turn);
                    state.turn_started = false;
                    self.active_turn_id
                        .store(turn.turn_id().get().get(), Ordering::Release);
                },
                Some(active) if active == *turn => {},
                Some(_) => return Err(AgentSessionError::TurnNoLongerActive),
            },
            AgentCommand::SteerTurn { turn, .. } | AgentCommand::InterruptTurn { turn }
                if state.active_turn != Some(*turn) =>
            {
                return Err(AgentSessionError::TurnNoLongerActive);
            },
            _ => {},
        }
        let response_request = match &command {
            AgentCommand::RespondToActivity { request, .. } => Some(*request),
            _ => None,
        };
        if response_request.is_some_and(|request| !state.outstanding_requests.contains(&request)) {
            return Err(AgentSessionError::NoOutstandingRequest);
        }
        if matches!(
            command,
            AgentCommand::SteerTurn { turn, .. }
                if state.interrupted_turns.contains(&turn)
        ) {
            return Err(AgentSessionError::TurnInterruptPending);
        }
        let interrupted = match &command {
            AgentCommand::InterruptTurn { turn } => Some(*turn),
            _ => None,
        };
        let urgent = response_request.is_some()
            || matches!(
                command,
                AgentCommand::InterruptTurn { turn }
                    if state.turn_started && state.active_turn == Some(turn)
            );
        let sender = if urgent {
            &self.urgent_commands
        } else {
            &self.commands
        };
        match sender.try_send(command) {
            Ok(()) => {
                if let Some(turn) = interrupted {
                    state.interrupted_turns.insert(turn);
                }
                if let Some(request) = response_request {
                    state.outstanding_requests.remove(&request);
                }
                Ok(CommandAdmission::Accepted)
            },
            Err(TrySendError::Full(command)) => Ok(CommandAdmission::Backpressured(
                PendingCommand::from_command(command),
            )),
            Err(TrySendError::Disconnected(_)) => Err(AgentSessionError::WorkerUnavailable(
                "the runtime worker closed before accepting an action".to_owned(),
            )),
        }
    }

    fn next_turn(&mut self) -> Result<TurnRef, AgentSessionError> {
        let id = NonZeroU64::new(self.next_turn_id).ok_or(AgentSessionError::TurnIdExhausted)?;
        self.next_turn_id = self.next_turn_id.checked_add(1).unwrap_or(0);
        Ok(TurnRef::new(self.session_id, TurnId::new(id)))
    }

    fn resolve_snapshot(&mut self, action: AgentIntent) -> Result<AgentCommand, AgentSessionError> {
        let active_turn = NonZeroU64::new(self.active_turn_id.load(Ordering::Acquire))
            .map(|id| TurnRef::new(self.session_id, TurnId::new(id)));
        match action {
            AgentIntent::Submit(text) => {
                let input = UserInput::new(text);
                if let Some(turn) = active_turn {
                    Ok(AgentCommand::SteerTurn { turn, input })
                } else {
                    Ok(AgentCommand::StartTurn {
                        turn: self.next_turn()?,
                        input,
                    })
                }
            },
            AgentIntent::Interrupt => Ok(AgentCommand::InterruptTurn {
                turn: active_turn.ok_or(AgentSessionError::NoActiveTurn)?,
            }),
            AgentIntent::RespondToApproval { request, decision } => {
                Ok(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::Approval(decision),
                })
            },
            AgentIntent::RespondToUserInput { request, input } => {
                Ok(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::UserInput(UserInput::new(input)),
                })
            },
        }
    }

    #[cfg(test)]
    fn wait_until_processed(&self, expected: u64) {
        let (processed, changed) = &*self.processed;
        let count = processed.lock().unwrap();
        let (count, timeout) = changed
            .wait_timeout_while(count, Duration::from_secs(1), |count| *count < expected)
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "worker did not process {expected} actions"
        );
        assert!(*count >= expected);
    }

    #[cfg(test)]
    fn wait_until_no_active_turn(&self) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let inactive = self
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .active_turn
                .is_none();
            if inactive {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "worker did not apply the Turn completion"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Queues one frontend intent without waiting for provider acceptance.
    pub fn dispatch(&mut self, action: AgentIntent) -> Result<CommandAdmission, AgentSessionError> {
        let state = Arc::clone(&self.state);
        let mut state = match state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                let command = self.resolve_snapshot(action)?;
                return Ok(CommandAdmission::Backpressured(
                    PendingCommand::from_command(command),
                ));
            },
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        let command = self.resolve_action(action, &mut state)?;
        self.queue_command(command, &mut state)
    }

    /// Retries an operation retained by an earlier dispatch attempt.
    pub fn retry(
        &mut self,
        pending: PendingCommand,
    ) -> Result<CommandAdmission, AgentSessionError> {
        let command = pending.into_command();
        let state = Arc::clone(&self.state);
        let mut state = match state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                return Ok(CommandAdmission::Backpressured(
                    PendingCommand::from_command(command),
                ));
            },
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        self.queue_command(command, &mut state)
    }

    /// Observes one already available semantic event without blocking.
    pub fn poll(&mut self) -> Result<RuntimePoll, AgentSessionError> {
        let Some(events) = self.events.as_ref() else {
            return Ok(RuntimePoll::Closed);
        };
        match events.try_recv() {
            Ok(WorkerEvent::Event(event)) => Ok(RuntimePoll::Event(event)),
            Ok(WorkerEvent::Failure(error)) => {
                if let Ok(mut failure) = self.failure.lock() {
                    failure.take();
                }
                Err(error)
            },
            Ok(WorkerEvent::Closed) => Ok(RuntimePoll::Closed),
            Err(TryRecvError::Empty) => Ok(RuntimePoll::Pending),
            Err(TryRecvError::Disconnected) => Ok(RuntimePoll::Closed),
        }
    }
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

enum WorkerEvent {
    Event(AgentEvent),
    Failure(AgentSessionError),
    Closed,
}

struct WorkerExit {
    terminal_events: Vec<AgentEvent>,
    failure: Option<AgentSessionError>,
}

impl WorkerExit {
    fn success() -> Self {
        Self {
            terminal_events: Vec::new(),
            failure: None,
        }
    }

    fn from_cleanup(result: Result<Vec<AgentEvent>, RuntimeError>) -> Self {
        match result {
            Ok(terminal_events) => Self {
                terminal_events,
                failure: None,
            },
            Err(error) => Self {
                terminal_events: Vec::new(),
                failure: Some(AgentSessionError::Runtime(error)),
            },
        }
    }
}

struct EventLane {
    sender: SyncSender<WorkerEvent>,
    pending_replaceable: Option<WorkerEvent>,
    failure: Arc<Mutex<Option<AgentSessionError>>>,
}

impl EventLane {
    fn new(
        sender: SyncSender<WorkerEvent>,
        failure: Arc<Mutex<Option<AgentSessionError>>>,
    ) -> Self {
        Self {
            sender,
            pending_replaceable: None,
            failure,
        }
    }

    fn send_events(&mut self, events: Vec<AgentEvent>) -> bool {
        events
            .into_iter()
            .all(|event| self.send(WorkerEvent::Event(event)))
    }

    fn send(&mut self, event: WorkerEvent) -> bool {
        if let WorkerEvent::Failure(error) = &event
            && let Ok(mut failure) = self.failure.lock()
        {
            *failure = Some(error.clone());
        }
        if is_replaceable(&event) {
            if let Some(pending) = self.pending_replaceable.as_mut()
                && same_replaceable_target(pending, &event)
            {
                *pending = event;
                return true;
            }
            if !self.flush_pending() {
                return false;
            }
            match self.sender.try_send(event) {
                Ok(()) => true,
                Err(TrySendError::Full(event)) => {
                    self.pending_replaceable = Some(event);
                    true
                },
                Err(TrySendError::Disconnected(_)) => false,
            }
        } else {
            self.flush_pending() && self.sender.send(event).is_ok()
        }
    }

    fn flush_pending(&mut self) -> bool {
        let Some(event) = self.pending_replaceable.take() else {
            return true;
        };
        self.sender.send(event).is_ok()
    }

    fn flush_available(&mut self) -> bool {
        let Some(event) = self.pending_replaceable.take() else {
            return true;
        };
        match self.sender.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(event)) => {
                self.pending_replaceable = Some(event);
                true
            },
            Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

struct AgentWorker<B> {
    runtime: AgentRuntime<B>,
    session_id: SessionId,
    state: Arc<Mutex<SessionState>>,
    active_turn_id: Arc<AtomicU64>,
}

impl<B: AgentBackend> AgentWorker<B> {
    fn new(
        backend: B,
        session_id: SessionId,
        state: Arc<Mutex<SessionState>>,
        active_turn_id: Arc<AtomicU64>,
    ) -> Self {
        Self {
            runtime: AgentRuntime::new(backend),
            session_id,
            state,
            active_turn_id,
        }
    }

    fn initialize(&mut self) -> Result<Vec<AgentEvent>, AgentSessionError> {
        match self.execute(AgentCommand::CreateSession {
            session_id: self.session_id,
        }) {
            Ok(events) => Ok(events),
            Err(start) => match self.runtime.shutdown() {
                Ok(_) => Err(AgentSessionError::Runtime(start)),
                Err(cleanup) => Err(AgentSessionError::StartAndCleanup {
                    start: Box::new(start),
                    cleanup: Box::new(cleanup),
                }),
            },
        }
    }

    fn run(
        &mut self,
        commands: Receiver<AgentCommand>,
        urgent_commands: Receiver<AgentCommand>,
        events: &mut EventLane,
        processed: &(Mutex<u64>, Condvar),
        lifecycle: &AtomicU8,
    ) -> WorkerExit {
        let mut deferred_commands = VecDeque::new();
        let mut deferred_urgent = VecDeque::new();
        loop {
            if !events.flush_available() {
                return WorkerExit::from_cleanup(self.runtime.shutdown());
            }
            if lifecycle.load(Ordering::Acquire) == WORKER_STOPPING {
                return WorkerExit::from_cleanup(self.runtime.shutdown());
            }

            let urgent = deferred_urgent
                .pop_front()
                .map_or_else(|| urgent_commands.try_recv(), Ok);
            let command = match urgent {
                Ok(command) => Ok(command),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => deferred_commands
                    .pop_front()
                    .map_or_else(|| commands.try_recv(), Ok),
            };
            match command {
                Ok(command) => {
                    if let AgentCommand::InterruptTurn { turn } = &command {
                        cancel_queued_turn_commands(*turn, &commands, &mut deferred_commands);
                        cancel_queued_turn_commands(*turn, &urgent_commands, &mut deferred_urgent);
                    }
                    if lifecycle
                        .compare_exchange(
                            WORKER_IDLE,
                            WORKER_EXECUTING,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        return WorkerExit::from_cleanup(self.runtime.shutdown());
                    }
                    let result = self.dispatch(command);
                    if let Ok(mut count) = processed.0.lock() {
                        *count += 1;
                        processed.1.notify_all();
                    }
                    let stopping = lifecycle
                        .compare_exchange(
                            WORKER_EXECUTING,
                            WORKER_IDLE,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err();
                    match result {
                        Ok(immediate) => {
                            if !events.send_events(immediate) {
                                return WorkerExit::from_cleanup(self.runtime.shutdown());
                            }
                        },
                        Err(error) => {
                            let _ = events.send(WorkerEvent::Failure(error));
                            return WorkerExit::from_cleanup(self.runtime.shutdown());
                        },
                    }
                    if stopping {
                        return WorkerExit::from_cleanup(self.runtime.shutdown());
                    }
                },
                Err(TryRecvError::Disconnected) => {
                    return WorkerExit::from_cleanup(self.runtime.shutdown());
                },
                Err(TryRecvError::Empty) => {},
            }

            match self.poll() {
                Ok(RuntimePoll::Pending) => {},
                Ok(RuntimePoll::Event(event)) => {
                    if !events.send(WorkerEvent::Event(event)) {
                        return WorkerExit::from_cleanup(self.runtime.shutdown());
                    }
                },
                Ok(RuntimePoll::Closed) => {
                    return match self.runtime.shutdown().map_err(AgentSessionError::Runtime) {
                        Ok(terminal) => {
                            let _ =
                                events.send_events(terminal) && events.send(WorkerEvent::Closed);
                            WorkerExit::success()
                        },
                        Err(error) => {
                            let _ = events.send(WorkerEvent::Failure(error));
                            WorkerExit::success()
                        },
                    };
                },
                Err(error) => {
                    let primary = AgentSessionError::Runtime(error);
                    let _ = events.send(WorkerEvent::Failure(primary));
                    return WorkerExit::from_cleanup(self.runtime.shutdown());
                },
            }

            thread::sleep(WORKER_POLL_INTERVAL);
        }
    }

    fn dispatch(&mut self, command: AgentCommand) -> Result<Vec<AgentEvent>, AgentSessionError> {
        self.execute(command).map_err(AgentSessionError::Runtime)
    }

    fn execute(&mut self, command: AgentCommand) -> Result<Vec<AgentEvent>, RuntimeError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let events = self.runtime.execute_command(command)?;
        apply_events(&mut state, &self.active_turn_id, &events);
        Ok(events)
    }

    fn poll(&mut self) -> Result<RuntimePoll, RuntimeError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let poll = self.runtime.poll_event()?;
        if let RuntimePoll::Event(event) = &poll {
            apply_event(&mut state, &self.active_turn_id, event);
        }
        Ok(poll)
    }
}

fn apply_events(state: &mut SessionState, active_turn_id: &AtomicU64, events: &[AgentEvent]) {
    for event in events {
        apply_event(state, active_turn_id, event);
    }
}

fn apply_event(state: &mut SessionState, active_turn_id: &AtomicU64, event: &AgentEvent) {
    match event {
        AgentEvent::ActivityStarted {
            activity,
            kind:
                ActivityKind::ApprovalRequest { request_id }
                | ActivityKind::UserInputRequest { request_id },
        } => {
            state
                .outstanding_requests
                .insert(ActivityRequestRef::new(*activity, *request_id));
        },
        AgentEvent::ActivityFinished { activity, .. } => {
            state
                .outstanding_requests
                .retain(|request| request.activity() != *activity);
        },
        AgentEvent::TurnStarted { turn } => {
            active_turn_id.store(turn.turn_id().get().get(), Ordering::Release);
            state.active_turn = Some(*turn);
            state.turn_started = true;
        },
        AgentEvent::TurnFinished { turn, .. } if state.active_turn == Some(*turn) => {
            active_turn_id.store(0, Ordering::Release);
            state.active_turn = None;
            state.turn_started = false;
            state.interrupted_turns.remove(turn);
        },
        _ => {},
    }
}

fn cancel_queued_turn_commands(
    interrupted: TurnRef,
    commands: &Receiver<AgentCommand>,
    retained: &mut VecDeque<AgentCommand>,
) {
    while let Ok(command) = commands.try_recv() {
        if command_turn(&command) != Some(interrupted) {
            retained.push_back(command);
        }
    }
}

fn command_turn(command: &AgentCommand) -> Option<TurnRef> {
    match command {
        AgentCommand::CreateSession { .. } => None,
        AgentCommand::StartTurn { turn, .. }
        | AgentCommand::SteerTurn { turn, .. }
        | AgentCommand::InterruptTurn { turn } => Some(*turn),
        AgentCommand::RespondToActivity { request, .. } => Some(request.activity().turn()),
    }
}

fn is_replaceable(event: &WorkerEvent) -> bool {
    matches!(
        event,
        WorkerEvent::Event(AgentEvent::ActivityUpdated {
            update: ActivityUpdate::TextSnapshot(_),
            ..
        })
    )
}

fn same_replaceable_target(left: &WorkerEvent, right: &WorkerEvent) -> bool {
    matches!(
        (left, right),
        (
            WorkerEvent::Event(AgentEvent::ActivityUpdated {
                activity: left,
                update: ActivityUpdate::TextSnapshot(_),
            }),
            WorkerEvent::Event(AgentEvent::ActivityUpdated {
                activity: right,
                update: ActivityUpdate::TextSnapshot(_),
            }),
        ) if left == right
    )
}

fn join_worker(worker: JoinHandle<WorkerExit>) -> Result<WorkerExit, AgentSessionError> {
    worker.join().map_err(|_| AgentSessionError::WorkerPanicked)
}

fn combine_failures(mut failures: Vec<AgentSessionError>) -> Option<AgentSessionError> {
    let mut combined = failures.pop()?;
    while let Some(primary) = failures.pop() {
        combined = AgentSessionError::Multiple {
            primary: Box::new(primary),
            additional: Box::new(combined),
        };
    }
    Some(combined)
}

/// Failure while starting, using, or stopping an [`AgentSession`].
#[derive(Clone, Debug)]
pub enum AgentSessionError {
    Runtime(RuntimeError),
    BackendCleanup(BackendFailure),
    StartAndCleanup {
        start: Box<RuntimeError>,
        cleanup: Box<RuntimeError>,
    },
    Multiple {
        primary: Box<AgentSessionError>,
        additional: Box<AgentSessionError>,
    },
    NoActiveTurn,
    NoOutstandingRequest,
    TurnNoLongerActive,
    TurnInterruptPending,
    TurnIdExhausted,
    WorkerUnavailable(String),
    WorkerShutdownTimedOut,
    WorkerPanicked,
}

impl fmt::Display for AgentSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => error.fmt(formatter),
            Self::BackendCleanup(error) => write!(formatter, "backend cleanup: {error}"),
            Self::StartAndCleanup { start, cleanup } => {
                write!(
                    formatter,
                    "{start}; additionally, startup cleanup failed: {cleanup}"
                )
            },
            Self::Multiple {
                primary,
                additional,
            } => {
                write!(formatter, "{primary}; additionally, {additional}")
            },
            Self::NoActiveTurn => formatter.write_str("no Turn is active"),
            Self::NoOutstandingRequest => {
                formatter.write_str("the Activity request is no longer outstanding")
            },
            Self::TurnNoLongerActive => {
                formatter.write_str("the command's target Turn is no longer active")
            },
            Self::TurnInterruptPending => {
                formatter.write_str("the active Turn is already being interrupted")
            },
            Self::TurnIdExhausted => formatter.write_str("the Turn identity space was exhausted"),
            Self::WorkerUnavailable(detail) => {
                write!(
                    formatter,
                    "the agent runtime worker is unavailable: {detail}"
                )
            },
            Self::WorkerShutdownTimedOut => {
                formatter.write_str("the agent runtime worker did not stop within 3 seconds")
            },
            Self::WorkerPanicked => formatter.write_str("the agent runtime worker panicked"),
        }
    }
}

impl Error for AgentSessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Runtime(error) => Some(error),
            Self::BackendCleanup(error) => Some(error),
            Self::StartAndCleanup { start, .. } => Some(start.as_ref()),
            Self::Multiple { primary, .. } => Some(primary),
            Self::NoActiveTurn
            | Self::NoOutstandingRequest
            | Self::TurnNoLongerActive
            | Self::TurnInterruptPending
            | Self::TurnIdExhausted
            | Self::WorkerUnavailable(_)
            | Self::WorkerShutdownTimedOut
            | Self::WorkerPanicked => None,
        }
    }
}

#[cfg(test)]
mod tests;
