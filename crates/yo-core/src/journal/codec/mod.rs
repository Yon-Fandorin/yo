//! Durable semantic Journal encoding and recovery.

mod correlation;
mod model;
mod recovery;
mod segmenter;
mod wire;

pub(crate) use model::{
    JournalCommit, JournalCommitKind, JournalRecord, MessageEnded, MessageOutcome, MessageReset,
    MessageSegment, MessageStream, MessageTerminal, ReplaySequence, SequencedJournalRecord,
};
pub(crate) use recovery::{RecoveredJournal, recover};
pub(crate) use segmenter::MessageSegmenter;
pub(crate) use wire::{JournalCodecError, decode, encode};

#[cfg(test)]
mod tests;
pub(crate) use correlation::{
    BackendBindingClosed, BackendBindingOpened, BackendExchangeObserved, BackendRequestAccepted,
    BackendResumableOutcome, BindingCloseReason, BindingTransition, CacheState, ContinuationAnchor,
    DetailAvailability, ExchangeDirection, ExchangeKind, ModelReplayDeltaRecord, OperationId,
    TransitionMode, VersionedIdentity,
};
