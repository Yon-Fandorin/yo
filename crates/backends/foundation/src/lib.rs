//! Transport-free contracts and evidence shared by Yo backend adapters.

mod contract;
mod evidence;

pub use contract::{
    BackendAdapter, BackendCapabilities, BackendFailure, BackendFailureKind, BackendPoll,
    BackendStopHandle,
};
pub use evidence::{
    BackendBindingEvidence, BackendCommandEvidence, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, ContinuationStrategy, ModelReplay, ModelReplayBudget,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ModelReplayTool,
    ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile,
};
#[doc(hidden)]
pub use evidence::{ProviderPrivateReplayPayload, validate_provider_private_replay_sequence};
