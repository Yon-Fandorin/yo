//! Shared contracts and bounded mechanisms for Yo backend adapters.

mod contract;
mod evidence;
mod image;
pub mod transport;

pub use contract::{
    BackendAdapter, BackendCapabilities, BackendFailure, BackendFailureKind, BackendPoll,
    BackendStopHandle, ImageInputCapability,
};
pub use evidence::{
    BackendBindingEvidence, BackendCommandEvidence, BackendIdentity, BackendOutcomeEvidence,
    BackendRequestEvidence, ContinuationStrategy, ModelReplay, ModelReplayBudget,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ModelReplayTool,
    ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile,
};
#[doc(hidden)]
pub use evidence::{ProviderPrivateReplayPayload, validate_provider_private_replay_sequence};
pub use image::{InputImageSnapshot, InputImageSnapshotError, ModelInputPart};
