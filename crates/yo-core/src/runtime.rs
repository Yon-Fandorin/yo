use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, OnceLock},
};

mod error;
mod execution;
mod poll;
mod replacement;
mod resume;

#[cfg(test)]
mod tests;

pub use error::RuntimeError;
#[cfg(test)]
use replacement::valid_replacement_binding;

use crate::{
    AgentBackend, AgentEngine, AgentEvent, BackendBindingEvidence, BackendEvent,
    BackendResumeSource, ContextPolicyChanged, ContinuationStrategy,
    InputAdmissionConfigurationError, InputAdmissionHost, InputImageHistory, JournalSequence,
    ModelReplay, SessionId, SubmissionId, TurnRef,
    journal::{ContextActiveSource, SessionJournal},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimePoll {
    Pending,
    Event(AgentEvent),
    Closed,
}

pub(crate) struct BackendReplacementError {
    pub(crate) primary: RuntimeError,
    pub(crate) cleanup_failure: Option<crate::BackendFailure>,
}

/// Composes one semantic engine with one initialized backend.
pub struct AgentRuntime<B> {
    engine: AgentEngine,
    backend: B,
    journal: SessionJournal,
    submission_ids: HashSet<SubmissionId>,
    input_admission: Arc<OnceLock<Box<dyn InputAdmissionHost>>>,
    input_admission_sealed: bool,
    binding_epoch: Option<u64>,
    binding: Option<BackendBindingEvidence>,
    continuation_strategy: Option<ContinuationStrategy>,
    model_replay: ModelReplay,
    replay_contract_rebind_required: bool,
    input_image_history: InputImageHistory,
    resume_source: Option<BackendResumeSource>,
    binding_has_accepted_request: bool,
    binding_has_unanchored_request: bool,
    context_policy: Option<ContextPolicyChanged>,
    context_epoch: Option<u64>,
    context_policy_initialized: bool,
    idle_context_compaction_pending: bool,
    idle_context_checkpoint_committed: bool,
    context_replay_groups: Vec<Vec<crate::ModelReplayItem>>,
    active_context_source: Option<ContextActiveSource>,
    accepted_requests: HashMap<TurnRef, JournalSequence>,
    accepted_submissions: HashMap<TurnRef, SubmissionId>,
    interview_start: Option<AgentEvent>,
    interview_delivery: VecDeque<AgentEvent>,
    interview_backend: Option<BackendEvent>,
    /// Once protected input reaches a backend call, later backend diagnostics may
    /// echo it and therefore remain redacted for this runtime's lifetime.
    secret_diagnostics_redacted: bool,
    /// A protected terminal input was accepted. Only its active Turn may finish.
    secret_input_terminal: bool,
}

impl<B: AgentBackend> AgentRuntime<B> {
    pub fn new(backend: B) -> Self {
        Self::with_journal(backend, SessionJournal::new())
    }

    pub(crate) fn with_journal(backend: B, journal: SessionJournal) -> Self {
        Self {
            engine: AgentEngine::new(),
            backend,
            journal,
            submission_ids: HashSet::new(),
            input_admission: Arc::new(OnceLock::new()),
            input_admission_sealed: false,
            binding_epoch: None,
            binding: None,
            continuation_strategy: None,
            model_replay: ModelReplay::default(),
            replay_contract_rebind_required: false,
            input_image_history: InputImageHistory::TextOnly,
            resume_source: None,
            binding_has_accepted_request: false,
            binding_has_unanchored_request: false,
            context_policy: None,
            context_epoch: None,
            context_policy_initialized: false,
            idle_context_compaction_pending: false,
            idle_context_checkpoint_committed: false,
            context_replay_groups: Vec::new(),
            active_context_source: None,
            accepted_requests: HashMap::new(),
            accepted_submissions: HashMap::new(),
            interview_start: None,
            interview_delivery: VecDeque::new(),
            interview_backend: None,
            secret_diagnostics_redacted: false,
            secret_input_terminal: false,
        }
    }

    /// Binds the Session's execution authority before any new input is submitted.
    pub fn configure_input_admission(
        &mut self,
        host: Box<dyn InputAdmissionHost>,
    ) -> Result<(), InputAdmissionConfigurationError> {
        if self.input_admission_sealed {
            return Err(InputAdmissionConfigurationError::InputAlreadySubmitted);
        }
        self.input_admission
            .set(host)
            .map_err(|_| InputAdmissionConfigurationError::AlreadyConfigured)
    }

    pub(crate) fn bind_input_admission(
        &mut self,
        slot: Arc<OnceLock<Box<dyn InputAdmissionHost>>>,
    ) {
        self.input_admission = slot;
    }

    pub fn session_id(&self) -> Option<SessionId> {
        self.engine.session_id()
    }

    pub fn active_turn(&self) -> Option<TurnRef> {
        self.engine.active_turn()
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn transcript_reader(&self) -> crate::TranscriptReader {
        self.journal.transcript_reader()
    }

    pub(crate) fn durability(&self) -> crate::JournalDurability {
        self.journal.transcript_reader().durability()
    }

    pub(super) fn redact_backend_failure(
        &self,
        failure: crate::BackendFailure,
    ) -> crate::BackendFailure {
        if self.secret_diagnostics_redacted {
            crate::BackendFailure::new(
                failure.kind(),
                "backend operation failed after protected input dispatch; details are withheld",
            )
        } else {
            failure
        }
    }

    pub(super) fn redact_backend_event(&self, event: BackendEvent) -> BackendEvent {
        if !self.secret_diagnostics_redacted {
            return event;
        }
        let redacted = || {
            crate::Failure::new(
                "backend operation failed after protected input dispatch; details are withheld",
            )
        };
        match event {
            BackendEvent::ActivityFinished {
                activity,
                outcome: crate::ActivityOutcome::Failed(_),
            } => BackendEvent::ActivityFinished {
                activity,
                outcome: crate::ActivityOutcome::Failed(redacted()),
            },
            BackendEvent::TurnFinished {
                turn,
                outcome: crate::TurnOutcome::Failed(_),
            } => BackendEvent::TurnFinished {
                turn,
                outcome: crate::TurnOutcome::Failed(redacted()),
            },
            event => event,
        }
    }

    pub(crate) const fn idle_context_compaction_pending(&self) -> bool {
        self.idle_context_compaction_pending
    }

    pub(crate) fn initialize_durability(&mut self) {
        self.journal.initialize_durability();
    }

    #[cfg(test)]
    pub(crate) fn journal(&self) -> &SessionJournal {
        &self.journal
    }
}
