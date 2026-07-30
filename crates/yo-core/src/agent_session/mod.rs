use std::{
    num::NonZeroU64,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    AgentBackend, AgentCommand, AgentEvent, BackendStopHandle, RuntimePoll, SessionId,
    TranscriptReader, journal::SessionJournal,
};

mod admission;
mod contract;
mod error;
mod worker;

use admission::SessionState;
pub use contract::{AgentIntent, CommandAdmission, PendingCommand};
pub use error::AgentSessionError;
#[cfg(test)]
use worker::apply_event;
use worker::{AgentWorker, EventLane, WorkerEvent, WorkerExit};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const WORKER_GRACEFUL_SHUTDOWN: Duration = Duration::from_millis(50);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const COMMAND_CAPACITY: usize = 32;
const URGENT_COMMAND_CAPACITY: usize = 8;
const EVENT_CAPACITY: usize = 256;
const WORKER_IDLE: u8 = 0;
const WORKER_EXECUTING: u8 = 1;
const WORKER_STOPPING: u8 = 2;

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
    transcript: TranscriptReader,
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
        let journal = SessionJournal::new();
        let transcript = journal.transcript_reader();
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
                    worker_state,
                    worker_active_turn_id,
                    journal,
                );
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
                    transcript,
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

    /// Returns read-only access to this Session's committed semantic history.
    #[must_use]
    pub fn transcript_reader(&self) -> TranscriptReader {
        self.transcript.clone()
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
