//! Durable semantic Journal encoding and recovery.

mod model;
mod recovery;
mod segmenter;
mod wire;

pub(crate) use model::{
    JournalCommit, JournalCommitFormat, JournalCommitKind, JournalRecord, MessageEnded,
    MessageOutcome, MessageReset, MessageSegment, MessageStream, MessageTerminal, ReplaySequence,
    SequencedJournalRecord,
};
pub(crate) use recovery::{RecoveredJournal, recover};
pub(crate) use segmenter::MessageSegmenter;
pub(crate) use wire::{JournalCodecError, decode, encode};

#[cfg(test)]
mod tests;
