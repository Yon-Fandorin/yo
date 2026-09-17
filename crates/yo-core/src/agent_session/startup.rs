use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64},
        mpsc,
    },
    thread,
    time::Instant,
};

use super::{
    AgentSession, AgentSessionError, AgentWorker, ChangeLane, SessionState,
    WORKER_SHUTDOWN_TIMEOUT, WorkerExit, WorkerSharedState,
};
use crate::{
    AgentBackend, BackendResumeTarget, SessionDescriptor, SessionId,
    journal::SessionJournal,
    readiness::{Readiness, ReadyReceiver},
    session_repository::{SessionRepository, StoredSessionContinuation},
};

pub(super) enum ResumeInitialization {
    SameBinding(BackendResumeTarget),
    Replacement(BackendResumeTarget),
}

impl AgentSession {
    /// Session을 시작하고 최초 backend handshake가 끝날 때까지 기다립니다.
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

    /// frontend host가 handshake를 취소할 수 있는 Session을 시작합니다.
    ///
    /// 취소가 요청되면 frontend가 Session의 소유권을 받기 전에
    /// backend 정리를 끝낸 뒤 `Ok(None)`을 반환합니다.
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

    /// 호출자가 소유한 identity로 비영속 Session을 시작합니다.
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

    /// `repository`를 통해 commit된 의미를 기록하는 Session을 시작합니다.
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

    /// 완전히 검증된 durable Session을 backend 작업을 만들거나 replay하지 않고 다시 엽니다.
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

    /// exact-replay Session을 durable model binding 교체와 함께 다시 엽니다.
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
        let (command_tx, command_rx) = mpsc::sync_channel(super::COMMAND_CAPACITY);
        let (urgent_tx, urgent_rx) = mpsc::sync_channel(super::URGENT_COMMAND_CAPACITY);
        let (replacement_tx, replacement_rx) = mpsc::sync_channel(super::REPLACEMENT_CAPACITY);
        let (change_tx, change_rx) = mpsc::sync_channel(super::CHANGE_CAPACITY);
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
        let lifecycle = Arc::new(AtomicU8::new(super::WORKER_IDLE));
        let worker_lifecycle = Arc::clone(&lifecycle);
        let transcript = journal.transcript_reader();
        let request_trace = journal.request_trace_reader();
        let submission_outcomes = Arc::new(Mutex::new(VecDeque::new()));
        let worker_submission_outcomes = Arc::clone(&submission_outcomes);
        let control_outcomes = Arc::new(Mutex::new(VecDeque::new()));
        let worker_control_outcomes = Arc::clone(&control_outcomes);
        let context_compaction_pending = Arc::new(AtomicBool::new(false));
        let worker_context_compaction_pending = Arc::clone(&context_compaction_pending);
        let readiness = Arc::new(Readiness::new());
        let worker_readiness = Arc::clone(&readiness);
        let input_admission = Arc::new(OnceLock::new());
        let worker_input_admission = Arc::clone(&input_admission);
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
                worker.runtime.bind_input_admission(worker_input_admission);
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
            let _ = super::join_worker(worker)?;
            return Err(cleanup.unwrap_or_else(|| {
                AgentSessionError::WorkerUnavailable(
                    "the runtime worker closed before receiving its backend".to_owned(),
                )
            }));
        }

        let mut terminated = false;
        let mut termination_started = None;
        let startup = loop {
            match startup_rx.recv_timeout(super::WORKER_POLL_INTERVAL) {
                Ok(startup) => break startup,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = super::join_worker(worker)?;
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
                    changes: Some(Mutex::new(ReadyReceiver::new(
                        change_rx,
                        Arc::clone(&readiness),
                    ))),
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
                    input_admission,
                    input_admission_sealed: false,
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
                let _ = super::join_worker(worker)?;
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
}
