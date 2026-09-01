use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread,
};

use super::{
    AgentControlOutcome, AgentSessionError, BackendReplacementOutcome, PendingCommand,
    ReplacementRequest, ResumeInitialization, SessionState, WORKER_EXECUTING, WORKER_IDLE,
    WORKER_POLL_INTERVAL, WORKER_STOPPING,
};
use crate::{
    ActivityKind, ActivityRequestRef, AgentBackend, AgentCommand, AgentEvent, AgentRuntime,
    RuntimeError, RuntimePoll, SessionId, SubmissionOutcome, SubmissionRejection,
    SubmissionRejectionKind, TurnRef, journal::SessionJournal,
};

pub(super) enum WorkerSignal {
    Changed,
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

pub(super) struct ChangeLane {
    sender: SyncSender<WorkerSignal>,
    failure: Arc<Mutex<Option<AgentSessionError>>>,
    readiness: Arc<crate::readiness::Readiness>,
}

impl ChangeLane {
    pub(super) fn new(
        sender: SyncSender<WorkerSignal>,
        failure: Arc<Mutex<Option<AgentSessionError>>>,
        readiness: Arc<crate::readiness::Readiness>,
    ) -> Self {
        Self {
            sender,
            failure,
            readiness,
        }
    }

    /// Publishes a level-triggered wake-up. One unread notification represents
    /// every committed suffix the reader has not consumed yet.
    pub(super) fn changed(&mut self) -> bool {
        let open = match self.sender.try_send(WorkerSignal::Changed) {
            Ok(()) | Err(TrySendError::Full(WorkerSignal::Changed)) => true,
            Err(TrySendError::Disconnected(_)) => false,
            Err(TrySendError::Full(_)) => {
                unreachable!("a terminal worker signal cannot precede another change")
            },
        };
        if open {
            self.readiness.notify();
        }
        open
    }

    pub(super) fn failure(&mut self, error: AgentSessionError) -> bool {
        if let Ok(mut failure) = self.failure.lock() {
            *failure = Some(error.clone());
        }
        let open = self.sender.send(WorkerSignal::Failure(error)).is_ok();
        if open {
            self.readiness.notify();
        }
        open
    }

    pub(super) fn close(&mut self) -> bool {
        let open = self.sender.send(WorkerSignal::Closed).is_ok();
        if open {
            self.readiness.notify();
        }
        open
    }
}

pub(super) struct AgentWorker {
    pub(super) runtime: AgentRuntime<Box<dyn AgentBackend + Send>>,
    session_id: SessionId,
    state: Arc<Mutex<SessionState>>,
    active_turn_id: Arc<AtomicU64>,
    submission_outcomes: Arc<Mutex<VecDeque<SubmissionOutcome>>>,
    control_outcomes: Arc<Mutex<VecDeque<AgentControlOutcome>>>,
    context_compaction_pending: Arc<AtomicBool>,
    resume: Option<ResumeInitialization>,
}

pub(super) struct WorkerSharedState {
    state: Arc<Mutex<SessionState>>,
    active_turn_id: Arc<AtomicU64>,
    submission_outcomes: Arc<Mutex<VecDeque<SubmissionOutcome>>>,
    control_outcomes: Arc<Mutex<VecDeque<AgentControlOutcome>>>,
    context_compaction_pending: Arc<AtomicBool>,
}

impl WorkerSharedState {
    pub(super) fn new(
        state: Arc<Mutex<SessionState>>,
        active_turn_id: Arc<AtomicU64>,
        submission_outcomes: Arc<Mutex<VecDeque<SubmissionOutcome>>>,
        control_outcomes: Arc<Mutex<VecDeque<AgentControlOutcome>>>,
        context_compaction_pending: Arc<AtomicBool>,
    ) -> Self {
        Self {
            state,
            active_turn_id,
            submission_outcomes,
            control_outcomes,
            context_compaction_pending,
        }
    }
}

impl AgentWorker {
    pub(super) fn new(
        backend: Box<dyn AgentBackend + Send>,
        session_id: SessionId,
        journal: SessionJournal,
        shared: WorkerSharedState,
        resume: Option<ResumeInitialization>,
    ) -> Self {
        Self {
            runtime: AgentRuntime::with_journal(backend, journal),
            session_id,
            state: shared.state,
            active_turn_id: shared.active_turn_id,
            submission_outcomes: shared.submission_outcomes,
            control_outcomes: shared.control_outcomes,
            context_compaction_pending: shared.context_compaction_pending,
            resume,
        }
    }

    pub(super) fn initialize(&mut self) -> Result<Vec<AgentEvent>, AgentSessionError> {
        if let Some(resume) = self.resume.take() {
            let initialized = match resume {
                ResumeInitialization::SameBinding(target) => {
                    self.runtime.initialize_resume(&target)
                },
                ResumeInitialization::Replacement(target) => {
                    self.runtime.initialize_resume_replacing_binding(&target)
                },
            };
            return match initialized {
                Ok(()) => Ok(Vec::new()),
                Err(start) => match self.runtime.shutdown() {
                    Ok(_) => Err(AgentSessionError::Runtime(start)),
                    Err(cleanup) => Err(AgentSessionError::StartAndCleanup {
                        start: Box::new(start),
                        cleanup: Box::new(cleanup),
                    }),
                },
            };
        }
        self.runtime.initialize_durability();
        match self.execute(
            AgentCommand::CreateSession {
                session_id: self.session_id,
            },
            None,
        ) {
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
        commands: Receiver<PendingCommand>,
        urgent_commands: Receiver<PendingCommand>,
        replacements: Receiver<ReplacementRequest>,
        changes: &mut ChangeLane,
        processed: &(Mutex<u64>, Condvar),
        lifecycle: &AtomicU8,
    ) -> WorkerExit {
        let mut deferred_commands = VecDeque::new();
        let mut deferred_urgent = VecDeque::new();
        let mut context_compaction_in_flight = false;
        loop {
            if lifecycle.load(Ordering::Acquire) == WORKER_STOPPING {
                return WorkerExit::from_cleanup(self.runtime.shutdown());
            }

            if !self.runtime.idle_context_compaction_pending() {
                match replacements.try_recv() {
                    Ok(request) => {
                        if lifecycle
                            .compare_exchange(
                                WORKER_IDLE,
                                WORKER_EXECUTING,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .is_err()
                        {
                            let mut backend = request.backend;
                            let cleanup = backend.shutdown().err();
                            let primary = AgentSessionError::WorkerUnavailable(
                                "binding replacement requires an idle Session".to_owned(),
                            );
                            let error = match cleanup {
                                Some(cleanup) => AgentSessionError::Multiple {
                                    primary: Box::new(primary),
                                    additional: Box::new(AgentSessionError::BackendCleanup(
                                        cleanup,
                                    )),
                                },
                                None => primary,
                            };
                            let _ = request.result.send(Err(error));
                            continue;
                        }
                        let durability_before = self.runtime.durability();
                        let result = self
                            .runtime
                            .replace_backend(request.backend)
                            .map(|cleanup_failure| BackendReplacementOutcome { cleanup_failure })
                            .map_err(|error| {
                                let primary = AgentSessionError::Runtime(error.primary);
                                match error.cleanup_failure {
                                    Some(cleanup) => AgentSessionError::Multiple {
                                        primary: Box::new(primary),
                                        additional: Box::new(AgentSessionError::BackendCleanup(
                                            cleanup,
                                        )),
                                    },
                                    None => primary,
                                }
                            });
                        lifecycle.store(WORKER_IDLE, Ordering::Release);
                        let changed =
                            self.runtime.durability() != durability_before || result.is_ok();
                        let _ = request.result.send(result);
                        if changed && !changes.changed() {
                            return WorkerExit::from_cleanup(self.runtime.shutdown());
                        }
                        continue;
                    },
                    Err(TryRecvError::Disconnected | TryRecvError::Empty) => {},
                }
            }

            let command = if self.runtime.idle_context_compaction_pending() {
                Err(TryRecvError::Empty)
            } else {
                let urgent = deferred_urgent
                    .pop_front()
                    .map_or_else(|| urgent_commands.try_recv(), Ok);
                match urgent {
                    Ok(command) => Ok(command),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => deferred_commands
                        .pop_front()
                        .map_or_else(|| commands.try_recv(), Ok),
                }
            };
            match command {
                Ok(pending) => {
                    if let AgentCommand::InterruptTurn { turn } = pending.command() {
                        let canceled =
                            cancel_queued_turn_commands(*turn, &commands, &mut deferred_commands)
                                .into_iter()
                                .chain(cancel_queued_turn_commands(
                                    *turn,
                                    &urgent_commands,
                                    &mut deferred_urgent,
                                ));
                        for id in canceled {
                            self.record_submission_outcome(SubmissionOutcome::Rejected {
                                id,
                                rejection: SubmissionRejection::new(
                                    SubmissionRejectionKind::TargetChanged,
                                    "the target Turn was interrupted before this submission ran",
                                ),
                            });
                        }
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
                    let (command, submission_id) = pending.into_parts();
                    let is_context_compaction =
                        matches!(&command, AgentCommand::CompactContext { .. });
                    let result = self.dispatch(command, submission_id);
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
                        Ok(_) => {
                            if is_context_compaction {
                                context_compaction_in_flight = true;
                            }
                            if let Some(id) = submission_id {
                                self.record_submission_outcome(SubmissionOutcome::Accepted { id });
                            }
                            if !changes.changed() {
                                return WorkerExit::from_cleanup(self.runtime.shutdown());
                            }
                        },
                        Err(error) => {
                            if is_context_compaction
                                && let Some(detail) = context_compaction_rejection(&error)
                            {
                                self.context_compaction_pending
                                    .store(false, Ordering::Release);
                                self.control_outcomes
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .push_back(AgentControlOutcome::ContextCompactionRejected {
                                        detail,
                                    });
                                if !changes.changed() {
                                    return WorkerExit::from_cleanup(self.runtime.shutdown());
                                }
                                continue;
                            }
                            return self.finish_after_failure(error, changes);
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

            let durability_before = self.runtime.durability();
            let poll = self.poll();
            let durability_changed = self.runtime.durability() != durability_before;
            match poll {
                Ok(RuntimePoll::Pending) => {
                    if durability_changed && !changes.changed() {
                        return WorkerExit::from_cleanup(self.runtime.shutdown());
                    }
                },
                Ok(RuntimePoll::Event(_)) => {
                    if !changes.changed() {
                        return WorkerExit::from_cleanup(self.runtime.shutdown());
                    }
                },
                Ok(RuntimePoll::Closed) => {
                    return match self.runtime.shutdown().map_err(AgentSessionError::Runtime) {
                        Ok(terminal) => {
                            let published = terminal.is_empty() || changes.changed();
                            let _ = published && changes.close();
                            WorkerExit::success()
                        },
                        Err(error) => {
                            let _ = changes.failure(error);
                            WorkerExit::success()
                        },
                    };
                },
                Err(error) => {
                    let primary = AgentSessionError::Runtime(error);
                    return self.finish_after_failure(primary, changes);
                },
            }

            if context_compaction_in_flight && !self.runtime.idle_context_compaction_pending() {
                context_compaction_in_flight = false;
                self.context_compaction_pending
                    .store(false, Ordering::Release);
                if !changes.changed() {
                    return WorkerExit::from_cleanup(self.runtime.shutdown());
                }
            }

            thread::sleep(WORKER_POLL_INTERVAL);
        }
    }

    fn record_submission_outcome(&self, outcome: SubmissionOutcome) {
        self.submission_outcomes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(outcome);
    }

    fn dispatch(
        &mut self,
        command: AgentCommand,
        submission_id: Option<crate::SubmissionId>,
    ) -> Result<Vec<AgentEvent>, AgentSessionError> {
        self.execute(command, submission_id)
            .map_err(AgentSessionError::Runtime)
    }

    fn execute(
        &mut self,
        command: AgentCommand,
        submission_id: Option<crate::SubmissionId>,
    ) -> Result<Vec<AgentEvent>, RuntimeError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let events = if let Some(submission_id) = submission_id {
            self.runtime.execute_submission(command, submission_id)?
        } else {
            self.runtime.execute_command(command)?
        };
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

    fn finish_after_failure(
        &mut self,
        primary: AgentSessionError,
        changes: &mut ChangeLane,
    ) -> WorkerExit {
        let primary_changed = matches!(
            &primary,
            AgentSessionError::Runtime(error) if !error.terminal_events().is_empty()
        );
        let cleanup = self.runtime.shutdown();
        let cleanup_changed = match &cleanup {
            Ok(events) => !events.is_empty(),
            Err(error) => !error.terminal_events().is_empty(),
        };
        if primary_changed || cleanup_changed {
            let _ = changes.changed();
        }
        let _ = changes.failure(primary);
        WorkerExit::from_cleanup(cleanup)
    }
}

fn context_compaction_rejection(error: &AgentSessionError) -> Option<String> {
    match error {
        AgentSessionError::Runtime(RuntimeError::CommandRejected(rejection)) => {
            Some(rejection.to_string())
        },
        AgentSessionError::Runtime(RuntimeError::Backend {
            failure,
            terminal_events,
        }) if failure.kind() == crate::BackendFailureKind::CommandRejected
            && terminal_events.is_empty() =>
        {
            Some(failure.message().to_owned())
        },
        AgentSessionError::ContextCompactionRequiresIdle => Some(error.to_string()),
        _ => None,
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
    commands: &Receiver<PendingCommand>,
    retained: &mut VecDeque<PendingCommand>,
) -> Vec<crate::SubmissionId> {
    let mut canceled_submissions = Vec::new();
    while let Ok(pending) = commands.try_recv() {
        if command_turn(pending.command()) != Some(interrupted) {
            retained.push_back(pending);
        } else if let Some(id) = pending.submission_id() {
            canceled_submissions.push(id);
        }
    }
    canceled_submissions
}

fn command_turn(command: &AgentCommand) -> Option<TurnRef> {
    match command {
        AgentCommand::CreateSession { .. } => None,
        AgentCommand::CompactContext { .. } => None,
        AgentCommand::StartTurn { turn, .. }
        | AgentCommand::SteerTurn { turn, .. }
        | AgentCommand::InterruptTurn { turn } => Some(*turn),
        AgentCommand::RespondToActivity { request, .. } => Some(request.activity().turn()),
    }
}
