//! Durable semantic Journal encoding and recovery.

mod context;
mod correlation;
mod fork;
mod model;
mod recovery;
mod segmenter;
mod wire;

pub(crate) use fork::{
    ForkExactReplay, ForkGroup, ForkHistoryCoordinate, ForkHistoryEntry, ForkItemCoordinate,
    ForkItemOrigin, ForkMessagePart, ForkSeed, ForkSource, ForkSourcePoint,
    INITIAL_FORK_SEED_PROFILE, InitialForkSeed,
};
pub(crate) use model::{
    JournalCommit, JournalCommitKind, JournalRecord, MessageEnded, MessageOutcome, MessageReset,
    MessageSegment, MessageStream, MessageTerminal, ReplaySequence, SequencedJournalRecord,
};
pub(crate) use recovery::{HistoricalForkKind, RecoveredJournal, recover};
pub(crate) use segmenter::MessageSegmenter;
pub(crate) use wire::{JournalCodecError, decode, encode, validate_image_input_encoding};

#[cfg(test)]
mod tests;
pub(crate) use context::{
    CONTEXT_ARTIFACT_PROFILE, CONTEXT_CHECKPOINT_PROFILE, CONTEXT_POLICY_PROFILE,
    ContextArtifactReceipt, ContextCheckpoint, ContextImageLoss, ContextImageSource, ContextLoss,
    ContextRetainedGroup, ContextSummaryUsage, IMAGE_CONTEXT_CHECKPOINT_PROFILE,
    validate_image_losses,
};
pub use context::{ContextPolicyChanged, ContextStrategy};
pub(crate) use correlation::{
    BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved, BackendRequestAccepted,
    BackendResumableOutcome, BindingCloseReason, BindingTransition, CacheState, ContinuationAnchor,
    DetailAvailability, ExchangeDirection, ExchangeKind, ModelReplayDeltaRecord, OperationId,
    TransitionMode, VersionedIdentity,
};
