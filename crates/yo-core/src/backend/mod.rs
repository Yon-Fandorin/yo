mod codex;
mod contract;
mod evidence;
mod native;
mod scripted;

pub use codex::{CodexBackend, CodexBackendConfig, CodexSkillReferenceProvider};
pub use contract::{
    AgentBackend, BackendCapabilities, BackendEvent, BackendFailure, BackendFailureKind,
    BackendPoll, BackendStopHandle,
};
pub use evidence::{
    BackendBindingEvidence, BackendCommandEvidence, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, BackendResumeTarget, ContinuationStrategy, ModelReplay,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ModelReplayTool,
    ReplayExecutor,
};
pub use native::{NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices};
pub use scripted::{BackendScriptStep, ScriptedBackend};
