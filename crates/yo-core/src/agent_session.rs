use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    task::{Context, Poll},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    AgentBackend, AgentEvent, BackendResumeTarget, BackendStopHandle, SessionDescriptor, SessionId,
    SubmissionOutcome, TranscriptReader,
    journal::SessionJournal,
    session_repository::{SessionRepository, StoredSessionContinuation},
};

mod admission;
mod contract;
mod error;
mod worker;

use admission::SessionState;
pub use contract::{AgentControlOutcome, AgentIntent, CommandAdmission, PendingCommand};
pub use error::AgentSessionError;
#[cfg(test)]
use worker::apply_event;
use worker::{AgentWorker, ChangeLane, WorkerExit, WorkerSharedState, WorkerSignal};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const WORKER_GRACEFUL_SHUTDOWN: Duration = Duration::from_millis(50);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const COMMAND_CAPACITY: usize = 32;
const URGENT_COMMAND_CAPACITY: usize = 8;
const CHANGE_CAPACITY: usize = 1;
const REPLACEMENT_CAPACITY: usize = 1;
const WORKER_IDLE: u8 = 0;
const WORKER_EXECUTING: u8 = 1;
const WORKER_STOPPING: u8 = 2;

enum ResumeInitialization {
    SameBinding(BackendResumeTarget),
    Replacement(BackendResumeTarget),
}

pub(super) struct ReplacementRequest {
    backend: Box<dyn AgentBackend + Send>,
    result: SyncSender<Result<BackendReplacementOutcome, AgentSessionError>>,
}

fn reject_replacement_after_cleanup(
    backend: &mut (dyn AgentBackend + Send),
    primary: AgentSessionError,
) -> AgentSessionError {
    match backend.shutdown() {
        Ok(()) => primary,
        Err(cleanup) => AgentSessionError::Multiple {
            primary: Box::new(primary),
            additional: Box::new(AgentSessionError::BackendCleanup(cleanup)),
        },
    }
}

/// Result of an atomically committed idle backend replacement.
#[derive(Clone, Debug)]
pub struct BackendReplacementOutcome {
    cleanup_failure: Option<crate::BackendFailure>,
}

impl BackendReplacementOutcome {
    /// Reports a previous-backend cleanup failure that occurred after the new epoch committed.
    #[must_use]
    pub const fn cleanup_failure(&self) -> Option<&crate::BackendFailure> {
        self.cleanup_failure.as_ref()
    }
}

/// Nonblocking frontend-independent connection to one worker-owned agent runtime.
pub struct AgentSession {
    commands: SyncSender<PendingCommand>,
    urgent_commands: SyncSender<PendingCommand>,
    replacements: SyncSender<ReplacementRequest>,
    changes: Option<Receiver<WorkerSignal>>,
    finished: Receiver<()>,
    stop: BackendStopHandle,
    failure: Arc<Mutex<Option<AgentSessionError>>>,
    lifecycle: Arc<AtomicU8>,
    session_id: SessionId,
    state: Arc<Mutex<SessionState>>,
    active_turn_id: Arc<AtomicU64>,
    next_turn_id: u64,
    transcript: TranscriptReader,
    request_trace: crate::RequestTraceReader,
    submission_outcomes: Arc<Mutex<VecDeque<SubmissionOutcome>>>,
    control_outcomes: Arc<Mutex<VecDeque<AgentControlOutcome>>>,
    context_compaction_pending: Arc<AtomicBool>,
    submission_ids: HashSet<crate::SubmissionId>,
    readiness: Arc<crate::readiness::Readiness>,
    #[cfg(test)]
    processed: Arc<(Mutex<u64>, Condvar)>,
    worker: Option<JoinHandle<WorkerExit>>,
}

/// Nonblocking status of the live Session's Journal and worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentSessionPoll {
    /// No Journal change or terminal worker state is currently available.
    Pending,
    /// One or more committed records may be read from [`TranscriptReader`].
    Changed,
    /// The worker closed after publishing every preceding Journal change.
    Closed,
}

impl AgentSession {
    /// Starts a Session and waits for its initial backend handshake.
    pub fn start<B>(backend: B) -> Result<Self, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        let mut never_cancelled = || false;
        Ok(Self::start_inner(
            backend,
            SessionId::new().map_err(AgentSessionError::SessionIdentityUnavailable)?,
            SessionJournal::new(),
            None,
            1,
            HashSet::new(),
            &mut never_cancelled,
        )?
        .expect("a callback that always returns false cannot cancel startup"))
    }

    #[cfg(test)]
    pub(crate) fn start_for_test<B>(
        backend: B,
        session_id: SessionId,
    ) -> Result<Self, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        Ok(
            Self::start_cancellable_with_id(backend, session_id, || false)?
                .expect("a callback that always returns false cannot cancel startup"),
        )
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
        Self::start_inner(
            backend,
            SessionId::new().map_err(AgentSessionError::SessionIdentityUnavailable)?,
            SessionJournal::new(),
            None,
            1,
            HashSet::new(),
            &mut is_cancelled,
        )
    }

    /// Starts a non-persistent Session with an identity owned by the caller.
    pub fn start_cancellable_with_id<B>(
        backend: B,
        session_id: SessionId,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        Self::start_inner(
            backend,
            session_id,
            SessionJournal::new(),
            None,
            1,
            HashSet::new(),
            &mut is_cancelled,
        )
    }

    /// Starts a Session whose committed semantics are written through `repository`.
    pub fn start_cancellable_with_repository<B, R>(
        backend: B,
        descriptor: SessionDescriptor,
        repository: R,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
        R: SessionRepository + Send + 'static,
    {
        let session_id = descriptor.session_id();
        Self::start_inner(
            backend,
            session_id,
            SessionJournal::with_repository_and_descriptor(Box::new(repository), descriptor),
            None,
            1,
            HashSet::new(),
            &mut is_cancelled,
        )
    }

    /// Reopens one fully validated durable Session without creating or replaying backend work.
    pub fn start_cancellable_with_continuation<B, R>(
        backend: B,
        continuation: StoredSessionContinuation,
        repository: R,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
        R: SessionRepository + Send + 'static,
    {
        let session_id = continuation.descriptor().session_id();
        let target = continuation.target().clone();
        let next_turn_id = continuation.next_turn_id();
        let submission_ids = continuation.submission_ids();
        let journal =
            SessionJournal::with_repository_and_continuation(Box::new(repository), &continuation);
        Self::start_inner(
            backend,
            session_id,
            journal,
            Some(ResumeInitialization::SameBinding(target)),
            next_turn_id,
            submission_ids,
            &mut is_cancelled,
        )
    }

    /// Reopens an exact-replay Session while replacing its durable model binding.
    pub fn start_cancellable_with_replacement_continuation<B, R>(
        backend: B,
        continuation: StoredSessionContinuation,
        repository: R,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
        R: SessionRepository + Send + 'static,
    {
        let session_id = continuation.descriptor().session_id();
        let target = continuation.target().clone();
        let next_turn_id = continuation.next_turn_id();
        let submission_ids = continuation.submission_ids();
        let journal =
            SessionJournal::with_repository_and_continuation(Box::new(repository), &continuation);
        Self::start_inner(
            backend,
            session_id,
            journal,
            Some(ResumeInitialization::Replacement(target)),
            next_turn_id,
            submission_ids,
            &mut is_cancelled,
        )
    }

    fn start_inner<B>(
        backend: B,
        session_id: SessionId,
        journal: SessionJournal,
        resume: Option<ResumeInitialization>,
        next_turn_id: u64,
        submission_ids: HashSet<crate::SubmissionId>,
        is_cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        let backend: Box<dyn AgentBackend + Send> = Box::new(backend);
        let stop = backend.stop_handle();
        let (command_tx, command_rx) = mpsc::sync_channel(COMMAND_CAPACITY);
        let (urgent_tx, urgent_rx) = mpsc::sync_channel(URGENT_COMMAND_CAPACITY);
        let (replacement_tx, replacement_rx) = mpsc::sync_channel(REPLACEMENT_CAPACITY);
        let (change_tx, change_rx) = mpsc::sync_channel(CHANGE_CAPACITY);
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
        let transcript = journal.transcript_reader();
        let request_trace = journal.request_trace_reader();
        let submission_outcomes = Arc::new(Mutex::new(VecDeque::new()));
        let worker_submission_outcomes = Arc::clone(&submission_outcomes);
        let control_outcomes = Arc::new(Mutex::new(VecDeque::new()));
        let worker_control_outcomes = Arc::clone(&control_outcomes);
        let context_compaction_pending = Arc::new(AtomicBool::new(false));
        let worker_context_compaction_pending = Arc::clone(&context_compaction_pending);
        let readiness = Arc::new(crate::readiness::Readiness::new());
        let worker_readiness = Arc::clone(&readiness);
        let worker = match thread::Builder::new()
            .name("yo-agent-runtime".to_owned())
            .spawn(move || {
                let Ok(backend) = backend_rx.recv() else {
                    let _ = finished_tx.send(());
                    return WorkerExit::success();
                };
                let mut worker = AgentWorker::new(
                    backend,
                    session_id,
                    journal,
                    WorkerSharedState::new(
                        worker_state,
                        worker_active_turn_id,
                        worker_submission_outcomes,
                        worker_control_outcomes,
                        worker_context_compaction_pending,
                    ),
                    resume,
                );
                let outcome = match worker.initialize() {
                    Ok(_) => {
                        if startup_tx.send(Ok(())).is_err() {
                            WorkerExit::from_cleanup(worker.runtime.shutdown())
                        } else {
                            let mut lane =
                                ChangeLane::new(change_tx, worker_failure, worker_readiness);
                            if !lane.changed() {
                                WorkerExit::from_cleanup(worker.runtime.shutdown())
                            } else {
                                worker.run(
                                    command_rx,
                                    urgent_rx,
                                    replacement_rx,
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
                    replacements: replacement_tx,
                    changes: Some(change_rx),
                    finished: finished_rx,
                    stop,
                    failure,
                    lifecycle,
                    session_id,
                    state,
                    active_turn_id,
                    next_turn_id,
                    transcript,
                    request_trace,
                    submission_outcomes,
                    control_outcomes,
                    context_compaction_pending,
                    submission_ids,
                    readiness,
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

    /// Atomically replaces the backend of an idle exact-replay Session.
    ///
    /// The current backend remains live if candidate resume or durable transition publication
    /// fails. Cancellation stops only the candidate while the worker completes that decision.
    pub fn replace_backend(
        &mut self,
        mut backend: Box<dyn AgentBackend + Send>,
        mut is_cancelled: impl FnMut() -> bool,
    ) -> Result<BackendReplacementOutcome, AgentSessionError> {
        if self.context_compaction_pending.load(Ordering::Acquire) {
            let primary = AgentSessionError::WorkerUnavailable(
                "binding replacement cannot overtake pending context compaction".to_owned(),
            );
            return Err(reject_replacement_after_cleanup(&mut *backend, primary));
        }
        if self.lifecycle.load(Ordering::Acquire) != WORKER_IDLE {
            let primary = AgentSessionError::WorkerUnavailable(
                "binding replacement requires an idle Session".to_owned(),
            );
            return Err(reject_replacement_after_cleanup(&mut *backend, primary));
        }
        let candidate_stop = backend.stop_handle();
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        if let Err(error) = self.replacements.try_send(ReplacementRequest {
            backend,
            result: result_tx,
        }) {
            let mut request = match error {
                TrySendError::Full(request) | TrySendError::Disconnected(request) => request,
            };
            let primary = AgentSessionError::WorkerUnavailable(
                "the backend replacement lane is unavailable".to_owned(),
            );
            return Err(reject_replacement_after_cleanup(
                &mut *request.backend,
                primary,
            ));
        }
        self.readiness.notify();
        let mut cancellation_requested = false;
        loop {
            match result_rx.recv_timeout(WORKER_POLL_INTERVAL) {
                Ok(result) => {
                    let outcome = result?;
                    self.stop = candidate_stop;
                    return Ok(outcome);
                },
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(AgentSessionError::WorkerUnavailable(
                        "the runtime worker closed during backend replacement".to_owned(),
                    ));
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if is_cancelled() && !cancellation_requested {
                        cancellation_requested = true;
                        candidate_stop.request_stop();
                    }
                },
            }
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
        if let Some(changes) = self.changes.take() {
            drop(changes);
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

    /// Observes whether committed Journal history or worker state changed.
    ///
    /// A `Changed` result carries no semantic data. Frontends read the
    /// committed suffix through [`Self::transcript_reader`].
    pub fn poll(&mut self) -> Result<AgentSessionPoll, AgentSessionError> {
        let Some(changes) = self.changes.as_ref() else {
            return Ok(AgentSessionPoll::Closed);
        };
        match changes.try_recv() {
            Ok(WorkerSignal::Changed) => Ok(AgentSessionPoll::Changed),
            Ok(WorkerSignal::Failure(error)) => {
                if let Ok(mut failure) = self.failure.lock() {
                    failure.take();
                }
                Err(error)
            },
            Ok(WorkerSignal::Closed) => Ok(AgentSessionPoll::Closed),
            Err(TryRecvError::Empty) => Ok(AgentSessionPoll::Pending),
            Err(TryRecvError::Disconnected) => Ok(AgentSessionPoll::Closed),
        }
    }

    /// Registers the frontend task that should observe the next Session change.
    pub fn poll_ready(&self, context: &mut Context<'_>) -> Poll<()> {
        self.readiness.poll(context)
    }

    /// Returns read-only access to this Session's committed semantic history.
    #[must_use]
    pub fn transcript_reader(&self) -> TranscriptReader {
        self.transcript.clone()
    }

    /// Returns read-only access to this Session's payload-free Request correlation trace.
    #[must_use]
    pub fn request_trace_reader(&self) -> crate::RequestTraceReader {
        self.request_trace.clone()
    }

    /// Takes the oldest whole-request admission result, if one is available.
    pub fn take_submission_outcome(&mut self) -> Option<SubmissionOutcome> {
        self.submission_outcomes.lock().ok()?.pop_front()
    }

    /// Takes the oldest completed control result, if one is available.
    pub fn take_control_outcome(&mut self) -> Option<AgentControlOutcome> {
        self.control_outcomes.lock().ok()?.pop_front()
    }
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
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

#[cfg(test)]
mod tests;
