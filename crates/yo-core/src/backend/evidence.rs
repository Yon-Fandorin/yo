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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendResumeTarget {
    session_id: crate::SessionId,
    epoch: u64,
    binding: BackendBindingEvidence,
    model_replay: ModelReplay,
    source_anchor_sequence: crate::JournalSequence,
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
            source_anchor_sequence,
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

    #[must_use]
    pub const fn source_anchor_sequence(&self) -> crate::JournalSequence {
        self.source_anchor_sequence
    }

    pub(crate) fn with_model_replay(mut self, replay: ModelReplay) -> Self {
        self.model_replay = replay;
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
