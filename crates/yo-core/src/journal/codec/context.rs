//! Context policy, retention, accounting, and checkpoint facade.

mod accounting;
mod checkpoint;
mod policy;
mod retention;

pub(crate) use accounting::ContextSummaryUsage;
pub(crate) use checkpoint::{
    CONTEXT_CHECKPOINT_PROFILE, ContextCheckpoint, IMAGE_CONTEXT_CHECKPOINT_PROFILE,
};
pub(crate) use policy::CONTEXT_POLICY_PROFILE;
pub use policy::{ContextPolicyChanged, ContextStrategy};
pub(crate) use retention::{
    CONTEXT_ARTIFACT_PROFILE, ContextArtifactReceipt, ContextImageLoss, ContextImageSource,
    ContextLoss, ContextRetainedGroup, validate_image_losses,
};
