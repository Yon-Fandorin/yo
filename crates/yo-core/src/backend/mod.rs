mod codex;
mod contract;
mod evidence;
mod native;
mod scripted;

pub use codex::{CodexBackend, CodexBackendConfig, CodexSkillReferenceProvider};
pub use contract::{
    AgentBackend, BackendAdapter, BackendCapabilities, BackendEvent, BackendFailure,
    BackendFailureKind, BackendPoll, BackendStopHandle,
};
pub use evidence::{
    BackendBindingEvidence, BackendCommandEvidence, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, BackendResumeTarget, ContinuationStrategy, ModelReplay,
    ModelReplayBudget, ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ModelReplayTool, ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile,
};
pub(crate) use evidence::{
    ProviderPrivateReplayPayload, provider_private_schema, replay_profile_id,
    validate_provider_private_replay_sequence,
};
pub use native::{
    ModelRequestObserver, NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
};
pub use scripted::{BackendScriptStep, ScriptedBackend};
