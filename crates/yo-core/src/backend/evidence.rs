pub use yo_backend::{
    BackendBindingEvidence, BackendCommandEvidence, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, ContinuationStrategy, ModelReplay, ModelReplayBudget,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ModelReplayTool,
    ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile,
};
pub(crate) use yo_backend::{
    ProviderPrivateReplayPayload, validate_provider_private_replay_sequence,
};

/// Durable Yo coordinates required to reconnect one existing backend binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendResumeSource {
    ContinuationAnchor(crate::JournalSequence),
    ContextCheckpoint(crate::JournalSequence),
}

impl BackendResumeSource {
    #[must_use]
    pub const fn sequence(self) -> crate::JournalSequence {
        match self {
            Self::ContinuationAnchor(sequence) | Self::ContextCheckpoint(sequence) => sequence,
        }
    }
}

/// Durable Yo coordinates required to reconnect one existing backend binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendResumeTarget {
    session_id: crate::SessionId,
    epoch: u64,
    binding: BackendBindingEvidence,
    model_replay: ModelReplay,
    replay_contract_rebind_required: bool,
    source: BackendResumeSource,
}

impl BackendResumeTarget {
    pub(crate) fn new(
        session_id: crate::SessionId,
        epoch: u64,
        binding: BackendBindingEvidence,
        source_anchor_sequence: crate::JournalSequence,
    ) -> Self {
        Self {
            session_id,
            epoch,
            binding,
            model_replay: ModelReplay::default(),
            replay_contract_rebind_required: false,
            source: BackendResumeSource::ContinuationAnchor(source_anchor_sequence),
        }
    }

    pub(crate) fn from_checkpoint(
        session_id: crate::SessionId,
        epoch: u64,
        binding: BackendBindingEvidence,
        source_checkpoint_sequence: crate::JournalSequence,
    ) -> Self {
        Self {
            session_id,
            epoch,
            binding,
            model_replay: ModelReplay::default(),
            replay_contract_rebind_required: false,
            source: BackendResumeSource::ContextCheckpoint(source_checkpoint_sequence),
        }
    }

    #[must_use]
    pub const fn session_id(&self) -> crate::SessionId {
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

    pub(crate) const fn replay_contract_rebind_required(&self) -> bool {
        self.replay_contract_rebind_required
    }

    #[must_use]
    pub const fn source(&self) -> BackendResumeSource {
        self.source
    }

    #[must_use]
    pub const fn source_anchor_sequence(&self) -> Option<crate::JournalSequence> {
        match self.source {
            BackendResumeSource::ContinuationAnchor(sequence) => Some(sequence),
            BackendResumeSource::ContextCheckpoint(_) => None,
        }
    }

    #[must_use]
    pub const fn source_checkpoint_sequence(&self) -> Option<crate::JournalSequence> {
        match self.source {
            BackendResumeSource::ContinuationAnchor(_) => None,
            BackendResumeSource::ContextCheckpoint(sequence) => Some(sequence),
        }
    }

    pub(crate) fn with_model_replay(mut self, replay: ModelReplay) -> Self {
        self.model_replay = replay;
        self
    }

    pub(crate) const fn with_replay_contract_rebind_required(mut self, required: bool) -> Self {
        self.replay_contract_rebind_required = required;
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
