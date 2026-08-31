mod contract;
mod evidence;
mod scripted;

pub use contract::{
    AgentBackend, BackendAdapter, BackendCapabilities, BackendEvent, BackendFailure,
    BackendFailureKind, BackendPoll, BackendStopHandle,
};
pub use evidence::{
    BackendBindingEvidence, BackendCommandEvidence, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, BackendResumeSource, BackendResumeTarget, ContinuationStrategy,
    ModelReplay, ModelReplayBudget, ModelReplayContract, ModelReplayDelta, ModelReplayItem,
    ModelReplayRole, ModelReplayTool, ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile,
    provider_private_schema,
};
pub(crate) use evidence::{
    ProviderPrivateReplayPayload, replay_profile_id, validate_provider_private_replay_sequence,
};
pub use scripted::{BackendScriptStep, ScriptedBackend};
