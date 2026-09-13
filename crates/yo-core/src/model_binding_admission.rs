//! Secret-free model binding admission injected by the composition root.

use crate::{CompleteModelBinding, ReasoningEffort};

/// Validates one exact complete binding before connection publication or runtime use.
///
/// Implementations must be deterministic and secret-free: admission does not perform
/// network requests or persistent writes. Composition must supply an implementation;
/// there is no permissive default or fallback for an unsupported binding.
pub trait ModelBindingAdmission: Send + Sync {
    fn admit(&self, complete: &CompleteModelBinding) -> Result<AdmittedCompleteBinding, String>;
}

impl<F> ModelBindingAdmission for F
where
    F: Fn(&CompleteModelBinding) -> Result<AdmittedCompleteBinding, String> + Send + Sync,
{
    fn admit(&self, complete: &CompleteModelBinding) -> Result<AdmittedCompleteBinding, String> {
        self(complete)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmittedToolPolicy {
    LocalTools,
    NoTools,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmittedModelProfile {
    reasoning_effort: Option<ReasoningEffort>,
    tool_policy: AdmittedToolPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmittedReplayProfile {
    SemanticOnly,
    ProviderPrivateLocalPlaintext,
}

/// Wire behavior selected only after validating the complete Chat service envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmittedChatImagePolicy {
    OpenRouterFreePng,
    QwenCloudGeneralPng,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmittedCompleteBinding {
    profile: AdmittedModelProfile,
    replay_profile: AdmittedReplayProfile,
    chat_image_policy: Option<AdmittedChatImagePolicy>,
}

impl AdmittedCompleteBinding {
    /// Reports the semantics supported by a trusted admission implementation.
    /// This DTO is not proof of validation: callers must invoke their mandatory
    /// admission port for the exact binding before using these semantics.
    pub const fn new(profile: AdmittedModelProfile, replay_profile: AdmittedReplayProfile) -> Self {
        Self {
            profile,
            replay_profile,
            chat_image_policy: None,
        }
    }

    pub const fn profile(self) -> AdmittedModelProfile {
        self.profile
    }

    pub const fn replay_profile(self) -> AdmittedReplayProfile {
        self.replay_profile
    }

    /// Reports a trusted validator's Chat image outcome; does not validate a binding.
    pub const fn with_chat_image_policy(mut self, policy: AdmittedChatImagePolicy) -> Self {
        self.chat_image_policy = Some(policy);
        self
    }

    pub const fn chat_image_policy(self) -> Option<AdmittedChatImagePolicy> {
        self.chat_image_policy
    }
}

impl AdmittedModelProfile {
    /// Constructs the neutral outcome of a trusted profile validator, not a validator itself.
    pub const fn new(
        reasoning_effort: Option<ReasoningEffort>,
        tool_policy: AdmittedToolPolicy,
    ) -> Self {
        Self {
            reasoning_effort,
            tool_policy,
        }
    }

    pub const fn reasoning_effort(self) -> Option<ReasoningEffort> {
        self.reasoning_effort
    }

    pub const fn tool_policy(self) -> AdmittedToolPolicy {
        self.tool_policy
    }
}
