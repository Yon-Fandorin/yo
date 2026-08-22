mod replay;

#[cfg(test)]
use replay::{MAX_REPLAY_CONTRACT_BYTES, MAX_REPLAY_DELTA_BYTES, MAX_REPLAY_TEXT_BYTES};
pub use replay::{
    ModelReplay, ModelReplayBudget, ModelReplayContract, ModelReplayDelta, ModelReplayItem,
    ModelReplayRole, ModelReplayTool, ProviderPrivateReplayEnvelope,
};
#[doc(hidden)]
pub use replay::{ProviderPrivateReplayPayload, validate_provider_private_replay_sequence};

/// Opaque provider-owned identity with an adapter-versioned interpretation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendIdentity {
    schema: String,
    value: String,
}

impl BackendIdentity {
    pub fn new(schema: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            value: value.into(),
        }
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    #[doc(hidden)]
    pub fn is_valid(&self) -> bool {
        valid_schema(&self.schema) && valid_value(&self.value)
    }
}

/// Adapter facts proving that a backend Session binding was created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendBindingEvidence {
    backend_kind: String,
    backend_version: String,
    binding_identity: BackendIdentity,
    model_identity: BackendIdentity,
    session_locator: BackendIdentity,
    continuation_strategy: ContinuationStrategy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayExecutor {
    LocalClient,
    ManagedServer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayProfile {
    SemanticOnly,
    ProviderPrivateLocalPlaintext,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuationStrategy {
    ExactReplay {
        executor: ReplayExecutor,
        replay_profile: ReplayProfile,
    },
    BackendManagedState,
}

impl BackendBindingEvidence {
    pub fn new(
        backend_kind: impl Into<String>,
        backend_version: impl Into<String>,
        binding_identity: BackendIdentity,
        model_identity: BackendIdentity,
        session_locator: BackendIdentity,
        continuation_strategy: ContinuationStrategy,
    ) -> Self {
        Self {
            backend_kind: backend_kind.into(),
            backend_version: backend_version.into(),
            binding_identity,
            model_identity,
            session_locator,
            continuation_strategy,
        }
    }

    pub fn backend_kind(&self) -> &str {
        &self.backend_kind
    }

    pub fn backend_version(&self) -> &str {
        &self.backend_version
    }

    pub const fn binding_identity(&self) -> &BackendIdentity {
        &self.binding_identity
    }

    pub const fn model_identity(&self) -> &BackendIdentity {
        &self.model_identity
    }

    pub const fn session_locator(&self) -> &BackendIdentity {
        &self.session_locator
    }

    pub const fn continuation_strategy(&self) -> ContinuationStrategy {
        self.continuation_strategy
    }

    #[doc(hidden)]
    pub fn same_resume_identity(&self, other: &Self) -> bool {
        self.backend_kind == other.backend_kind
            && self.binding_identity == other.binding_identity
            && self.model_identity == other.model_identity
            && self.session_locator == other.session_locator
            && self.continuation_strategy == other.continuation_strategy
    }

    #[doc(hidden)]
    pub fn is_valid(&self) -> bool {
        valid_schema(&self.backend_kind)
            && valid_value(&self.backend_version)
            && self.binding_identity.is_valid()
            && self.model_identity.is_valid()
            && self.session_locator.is_valid()
    }
}

/// Adapter facts proving that one backend request was sent and accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendRequestEvidence {
    payload_schema: String,
    exchange_identity: BackendIdentity,
    request_identity: BackendIdentity,
}

impl BackendRequestEvidence {
    pub fn new(
        payload_schema: impl Into<String>,
        exchange_identity: BackendIdentity,
        request_identity: BackendIdentity,
    ) -> Self {
        Self {
            payload_schema: payload_schema.into(),
            exchange_identity,
            request_identity,
        }
    }

    pub fn payload_schema(&self) -> &str {
        &self.payload_schema
    }

    pub const fn exchange_identity(&self) -> &BackendIdentity {
        &self.exchange_identity
    }

    pub const fn request_identity(&self) -> &BackendIdentity {
        &self.request_identity
    }

    #[doc(hidden)]
    pub fn is_valid(&self) -> bool {
        valid_schema(&self.payload_schema)
            && self.exchange_identity.is_valid()
            && self.request_identity.is_valid()
    }
}

fn valid_schema(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.is_ascii()
}

fn valid_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096
}

/// Provider-neutral evidence returned after command acceptance.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum BackendCommandEvidence {
    #[default]
    None,
    BindingOpened(BackendBindingEvidence),
    RequestAccepted(BackendRequestEvidence),
}

/// Provider evidence that a completed Turn is stable enough to resume.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BackendOutcomeEvidence {
    outcome_identity: Option<BackendIdentity>,
    model_replay: Option<ModelReplayDelta>,
}

impl BackendOutcomeEvidence {
    #[must_use]
    pub const fn without_identity() -> Self {
        Self {
            outcome_identity: None,
            model_replay: None,
        }
    }

    #[must_use]
    pub fn with_identity(identity: BackendIdentity) -> Self {
        Self {
            outcome_identity: Some(identity),
            model_replay: None,
        }
    }

    #[must_use]
    pub fn with_replay(mut self, replay: ModelReplayDelta) -> Self {
        self.model_replay = Some(replay);
        self
    }

    pub const fn outcome_identity(&self) -> Option<&BackendIdentity> {
        self.outcome_identity.as_ref()
    }

    pub const fn model_replay(&self) -> Option<&ModelReplayDelta> {
        self.model_replay.as_ref()
    }

    #[doc(hidden)]
    pub fn is_valid(&self) -> bool {
        self.outcome_identity
            .as_ref()
            .is_none_or(BackendIdentity::is_valid)
            && self
                .model_replay
                .as_ref()
                .is_none_or(ModelReplayDelta::is_valid)
    }
}

#[cfg(test)]
mod tests;
