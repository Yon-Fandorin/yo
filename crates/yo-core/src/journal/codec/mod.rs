//! Durable semantic Journal encoding and recovery.

mod model;
mod recovery;
mod segmenter;
mod wire;

pub(crate) use model::{
    JournalCommit, JournalCommitKind, JournalRecord, MessageEnded, MessageOutcome, MessageSegment,
    MessageStream, MessageTerminal, SequencedJournalRecord,
};
pub(crate) use recovery::{RecoveredJournal, recover};
#[cfg(test)]
pub(crate) use segmenter::MessageSegmenter;
pub(crate) use wire::{JournalCodecError, decode, encode};

#[cfg(test)]
mod tests;
