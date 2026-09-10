pub use yo_backend::{
    BackendBindingEvidence, BackendCommandEvidence, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, ContinuationStrategy, ModelReplay, ModelReplayBudget,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ModelReplayTool,
    ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile,
};
pub(crate) use yo_backend::{
    ProviderPrivateReplayPayload, validate_provider_private_replay_sequence,
};

use crate::{ContextPolicyChanged, JournalSequence, SessionId};

/// Durable evidence about whether a resumed backend binding may contain image input.
///
/// `Unknown` is deliberately conservative: a displayed transcript or model replay is not enough
/// to prove that an inherited backend archive was text-only.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputImageHistory {
    #[default]
    Unknown,
    TextOnly,
    ContainsImages,
}

/// Durable Yo coordinates required to reconnect one existing backend binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendResumeSource {
    ContinuationAnchor(JournalSequence),
    ContextCheckpoint(JournalSequence),
    InitialFork(JournalSequence),
}

impl BackendResumeSource {
    #[must_use]
    pub const fn sequence(self) -> JournalSequence {
        match self {
            Self::ContinuationAnchor(sequence)
            | Self::ContextCheckpoint(sequence)
            | Self::InitialFork(sequence) => sequence,
        }
    }
}

/// Durable Yo coordinates required to reconnect one existing backend binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendResumeTarget {
    session_id: SessionId,
    epoch: u64,
    binding: BackendBindingEvidence,
    model_replay: ModelReplay,
    model_replay_groups: Vec<Vec<ModelReplayItem>>,
    context_policy: Option<ContextPolicyChanged>,
    context_epoch: Option<u64>,
    replay_contract_rebind_required: bool,
    binding_has_accepted_request: bool,
    input_image_history: InputImageHistory,
    source: Option<BackendResumeSource>,
}

impl BackendResumeTarget {
    pub(crate) fn new(
        session_id: SessionId,
        epoch: u64,
        binding: BackendBindingEvidence,
        source_anchor_sequence: JournalSequence,
    ) -> Self {
        Self {
            session_id,
            epoch,
            binding,
            model_replay: ModelReplay::default(),
            model_replay_groups: Vec::new(),
            context_policy: None,
            context_epoch: None,
            replay_contract_rebind_required: false,
            binding_has_accepted_request: true,
            input_image_history: InputImageHistory::Unknown,
            source: Some(BackendResumeSource::ContinuationAnchor(
                source_anchor_sequence,
            )),
        }
    }

    pub(crate) fn from_checkpoint(
        session_id: SessionId,
        epoch: u64,
        binding: BackendBindingEvidence,
        source_checkpoint_sequence: JournalSequence,
    ) -> Self {
        Self {
            session_id,
            epoch,
            binding,
            model_replay: ModelReplay::default(),
            model_replay_groups: Vec::new(),
            context_policy: None,
            context_epoch: None,
            replay_contract_rebind_required: false,
            binding_has_accepted_request: true,
            input_image_history: InputImageHistory::Unknown,
            source: Some(BackendResumeSource::ContextCheckpoint(
                source_checkpoint_sequence,
            )),
        }
    }

    pub(crate) fn from_initial_fork(
        session_id: SessionId,
        epoch: u64,
        binding: BackendBindingEvidence,
        source_initial_fork_sequence: JournalSequence,
    ) -> Self {
        Self {
            session_id,
            epoch,
            binding,
            model_replay: ModelReplay::default(),
            model_replay_groups: Vec::new(),
            context_policy: None,
            context_epoch: None,
            replay_contract_rebind_required: false,
            binding_has_accepted_request: false,
            input_image_history: InputImageHistory::Unknown,
            source: Some(BackendResumeSource::InitialFork(
                source_initial_fork_sequence,
            )),
        }
    }

    pub(crate) fn for_model_rebind(
        session_id: SessionId,
        epoch: u64,
        binding: BackendBindingEvidence,
        source_anchor_sequence: Option<JournalSequence>,
    ) -> Self {
        Self {
            session_id,
            epoch,
            binding,
            model_replay: ModelReplay::default(),
            model_replay_groups: Vec::new(),
            context_policy: None,
            context_epoch: None,
            replay_contract_rebind_required: false,
            binding_has_accepted_request: false,
            input_image_history: InputImageHistory::Unknown,
            source: source_anchor_sequence.map(BackendResumeSource::ContinuationAnchor),
        }
    }

    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    #[must_use]
    pub const fn binding(&self) -> &BackendBindingEvidence {
        &self.binding
    }

    #[must_use]
    pub const fn model_replay(&self) -> &ModelReplay {
        &self.model_replay
    }

    #[must_use]
    pub fn model_replay_groups(&self) -> &[Vec<ModelReplayItem>] {
        &self.model_replay_groups
    }

    #[must_use]
    pub const fn context_policy(&self) -> Option<&ContextPolicyChanged> {
        self.context_policy.as_ref()
    }

    #[must_use]
    pub const fn context_epoch(&self) -> Option<u64> {
        self.context_epoch
    }

    pub(crate) const fn replay_contract_rebind_required(&self) -> bool {
        self.replay_contract_rebind_required
    }

    pub(crate) const fn binding_has_accepted_request(&self) -> bool {
        self.binding_has_accepted_request
    }

    #[must_use]
    pub const fn input_image_history(&self) -> InputImageHistory {
        self.input_image_history
    }

    #[must_use]
    pub const fn source(&self) -> Option<BackendResumeSource> {
        self.source
    }

    #[must_use]
    pub const fn source_anchor_sequence(&self) -> Option<JournalSequence> {
        match self.source {
            Some(BackendResumeSource::ContinuationAnchor(sequence)) => Some(sequence),
            Some(
                BackendResumeSource::ContextCheckpoint(_) | BackendResumeSource::InitialFork(_),
            )
            | None => None,
        }
    }

    #[must_use]
    pub const fn source_checkpoint_sequence(&self) -> Option<JournalSequence> {
        match self.source {
            Some(BackendResumeSource::ContextCheckpoint(sequence)) => Some(sequence),
            Some(
                BackendResumeSource::ContinuationAnchor(_) | BackendResumeSource::InitialFork(_),
            )
            | None => None,
        }
    }

    #[must_use]
    pub const fn source_initial_fork_sequence(&self) -> Option<JournalSequence> {
        match self.source {
            Some(BackendResumeSource::InitialFork(sequence)) => Some(sequence),
            Some(
                BackendResumeSource::ContinuationAnchor(_)
                | BackendResumeSource::ContextCheckpoint(_),
            )
            | None => None,
        }
    }

    pub(crate) fn with_model_replay(mut self, replay: ModelReplay) -> Self {
        self.model_replay = replay;
        self
    }

    pub(crate) fn with_context_state(
        mut self,
        policy: Option<ContextPolicyChanged>,
        context_epoch: Option<u64>,
        replay_groups: Vec<Vec<ModelReplayItem>>,
    ) -> Self {
        self.context_policy = policy;
        self.context_epoch = context_epoch;
        self.model_replay_groups = replay_groups;
        self
    }

    pub(crate) const fn with_replay_contract_rebind_required(mut self, required: bool) -> Self {
        self.replay_contract_rebind_required = required;
        self
    }

    pub(crate) const fn with_binding_has_accepted_request(mut self, accepted: bool) -> Self {
        self.binding_has_accepted_request = accepted;
        self
    }

    pub(crate) const fn with_input_image_history(mut self, history: InputImageHistory) -> Self {
        self.input_image_history = history;
        self
    }
}

pub(crate) const fn replay_profile_id(profile: ReplayProfile) -> &'static str {
    match profile {
        ReplayProfile::SemanticOnly => crate::SEMANTIC_REPLAY_PROFILE,
        ReplayProfile::ProviderPrivateLocalPlaintext => crate::KIMI_PRIVATE_REPLAY_PROFILE,
    }
}

/// Returns the exact opaque replay schema admitted for a shared replay profile.
#[must_use]
pub const fn provider_private_schema(profile: ReplayProfile) -> Option<&'static str> {
    match profile {
        ReplayProfile::SemanticOnly => None,
        ReplayProfile::ProviderPrivateLocalPlaintext => Some("kimi.assistant-message/v1alpha1"),
    }
}

#[cfg(test)]
mod tests;
