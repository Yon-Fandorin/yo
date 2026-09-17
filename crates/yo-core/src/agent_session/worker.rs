use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64},
    },
};

use super::{AgentControlOutcome, SessionState};
use crate::{AgentBackend, AgentRuntime, SessionId, SubmissionOutcome, journal::SessionJournal};

mod commands;
mod events;
mod lifecycle;
mod outcomes;
mod signals;

pub(super) use commands::cancel_queued_turn_commands;
#[cfg(test)]
pub(super) use events::apply_event;
pub(super) use events::apply_events;
pub(super) use outcomes::{context_compaction_rejection, submission_rejection};
pub(super) use signals::{ChangeLane, WorkerExit, WorkerSignal};

pub(super) use super::ReplacementRequest;

/// worker가 공유하는 Session 상태 묶음입니다.
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

/// backend runtime와 Session 상태를 하나의 worker 구성으로 묶습니다.
pub(super) struct AgentWorker {
    pub(super) runtime: AgentRuntime<Box<dyn AgentBackend + Send>>,
    session_id: SessionId,
    state: Arc<Mutex<SessionState>>,
    active_turn_id: Arc<AtomicU64>,
    submission_outcomes: Arc<Mutex<VecDeque<SubmissionOutcome>>>,
    control_outcomes: Arc<Mutex<VecDeque<AgentControlOutcome>>>,
    context_compaction_pending: Arc<AtomicBool>,
    resume: Option<super::ResumeInitialization>,
}

impl AgentWorker {
    pub(super) fn new(
        backend: Box<dyn AgentBackend + Send>,
        session_id: SessionId,
        journal: SessionJournal,
        shared: WorkerSharedState,
        resume: Option<super::ResumeInitialization>,
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
}
