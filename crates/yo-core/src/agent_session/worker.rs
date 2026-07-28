use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
};

use super::{
    AgentSessionError, SessionState, WORKER_EXECUTING, WORKER_IDLE, WORKER_POLL_INTERVAL,
    WORKER_STOPPING,
};
use crate::{
    ActivityKind, ActivityRequestRef, ActivityUpdate, AgentBackend, AgentCommand, AgentEvent,
    AgentRuntime, RuntimeError, RuntimePoll, SessionId, TurnRef,
};

pub(super) enum WorkerEvent {
    Event(AgentEvent),
    Failure(AgentSessionError),
    Closed,
}

pub(super) struct WorkerExit {
    pub(super) terminal_events: Vec<AgentEvent>,
    pub(super) failure: Option<AgentSessionError>,
}

impl WorkerExit {
    pub(super) fn success() -> Self {
        Self {
            terminal_events: Vec::new(),
            failure: None,
        }
    }

    pub(super) fn from_cleanup(result: Result<Vec<AgentEvent>, RuntimeError>) -> Self {
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

pub(super) struct EventLane {
    sender: SyncSender<WorkerEvent>,
    pending_replaceable: Option<WorkerEvent>,
    failure: Arc<Mutex<Option<AgentSessionError>>>,
}

impl EventLane {
    pub(super) fn new(
        sender: SyncSender<WorkerEvent>,
        failure: Arc<Mutex<Option<AgentSessionError>>>,
    ) -> Self {
        Self {
            sender,
            pending_replaceable: None,
            failure,
        }
    }

    pub(super) fn send_events(&mut self, events: Vec<AgentEvent>) -> bool {
        events
            .into_iter()
            .all(|event| self.send(WorkerEvent::Event(event)))
    }

    pub(super) fn send(&mut self, event: WorkerEvent) -> bool {
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

    pub(super) fn flush_available(&mut self) -> bool {
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

pub(super) struct AgentWorker<B> {
    pub(super) runtime: AgentRuntime<B>,
    session_id: SessionId,
    state: Arc<Mutex<SessionState>>,
    active_turn_id: Arc<AtomicU64>,
}

impl<B: AgentBackend> AgentWorker<B> {
    pub(super) fn new(
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

    pub(super) fn initialize(&mut self) -> Result<Vec<AgentEvent>, AgentSessionError> {
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

    pub(super) fn run(
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

pub(super) fn apply_event(
    state: &mut SessionState,
    active_turn_id: &AtomicU64,
    event: &AgentEvent,
) {
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
