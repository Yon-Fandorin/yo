mod context;
mod contract;
mod evidence;
mod scripted;

#[cfg(test)]
mod tests;

pub use context::{
    ContextAccounting, ContextAccountingQuality, ContextCheckpointProposal,
    ContextPressureDecision, ContextPressureObservation, KIMI_CODE_IMAGE_ACCOUNTING_PROFILE,
};
pub use contract::{
    AgentBackend, BackendAdapter, BackendCapabilities, BackendEvent, BackendFailure,
    BackendFailureKind, BackendPoll, BackendStopHandle, ImageInputCapability,
};
pub use evidence::{
    BackendBindingEvidence, BackendCommandEvidence, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, BackendResumeSource, BackendResumeTarget, ContinuationStrategy,
    InputImageHistory, ModelReplay, ModelReplayBudget, ModelReplayContract, ModelReplayDelta,
    ModelReplayItem, ModelReplayRole, ModelReplayTool, ProviderPrivateReplayEnvelope,
    ReplayExecutor, ReplayProfile, provider_private_schema,
};
pub(crate) use evidence::{
    ProviderPrivateReplayPayload, replay_profile_id, validate_provider_private_replay_sequence,
};
pub use scripted::{BackendScriptStep, ScriptedBackend};
pub use yo_backend::{InputImageSnapshot, InputImageSnapshotError, ModelInputPart};
