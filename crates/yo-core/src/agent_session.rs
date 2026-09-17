use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64},
        mpsc::{Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

use crate::{
    BackendStopHandle, InputAdmissionHost, SessionId, SubmissionOutcome, TranscriptReader,
    readiness::{Readiness, ReadyReceiver},
};

mod admission;
mod contract;
mod error;
mod fork;
mod lifecycle;
mod observation;
mod replacement;
mod startup;
mod worker;

use admission::SessionState;
pub use contract::{AgentControlOutcome, AgentIntent, CommandAdmission, PendingCommand};
pub use error::AgentSessionError;
pub(super) use lifecycle::join_worker;
pub use observation::AgentSessionPoll;
pub use replacement::BackendReplacementOutcome;
pub(super) use replacement::ReplacementRequest;
pub(super) use startup::ResumeInitialization;
#[cfg(test)]
pub(super) use worker::apply_event;
pub(super) use worker::{AgentWorker, ChangeLane, WorkerExit, WorkerSharedState, WorkerSignal};

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

/// 한 worker가 소유하는 비차단 AgentSession 연결의 공유 상태입니다.
pub struct AgentSession {
    commands: SyncSender<PendingCommand>,
    urgent_commands: SyncSender<PendingCommand>,
    replacements: SyncSender<replacement::ReplacementRequest>,
    changes: Option<Mutex<ReadyReceiver<WorkerSignal>>>,
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
    input_admission: Arc<OnceLock<Box<dyn InputAdmissionHost>>>,
    input_admission_sealed: bool,
    readiness: Arc<Readiness>,
    #[cfg(test)]
    processed: Arc<(Mutex<u64>, Condvar)>,
    worker: Option<JoinHandle<WorkerExit>>,
}

#[cfg(test)]
mod tests;
