use std::{
    fmt,
    sync::{Arc, RwLock},
};

use super::{JournalEntry, JournalSequence, SemanticRecord, SessionJournalState, read_state};
use crate::{AgentCommand, AgentEvent};

const READ_LIMIT: usize = 256;

/// Read-only access to the semantic history of one live Session.
///
/// The reader copies a bounded suffix and never exposes the Journal's lock or
/// storage layout. A future remote implementation can therefore preserve the
/// same sequence-based contract without copying the local implementation.
/// Keeping a reader alive also retains the in-memory Session history it reads.
#[derive(Clone)]
pub struct TranscriptReader {
    pub(super) state: Arc<RwLock<SessionJournalState>>,
}

impl fmt::Debug for TranscriptReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TranscriptReader")
            .finish_non_exhaustive()
    }
}

impl TranscriptReader {
    /// Copies the next bounded group of records after `sequence`.
    ///
    /// `None` starts before the first record. Callers continue by passing the
    /// sequence of the last returned entry.
    #[must_use]
    pub fn read_after(&self, sequence: Option<JournalSequence>) -> TranscriptSlice {
        let state = read_state(&self.state);
        let head = state.entries.last().map(JournalEntry::sequence);
        let start = sequence.map_or(0, |sequence| {
            usize::try_from(sequence.get())
                .unwrap_or(usize::MAX)
                .min(state.entries.len())
        });
        let entries = state
            .entries
            .iter()
            .skip(start)
            .take(READ_LIMIT)
            .map(TranscriptEntry::from)
            .collect::<Vec<_>>();
        drop(state);

        TranscriptSlice {
            from: entries.first().map(TranscriptEntry::sequence),
            head,
            entries,
        }
    }

    /// Returns the newest available sequence without copying records.
    #[must_use]
    pub fn head_sequence(&self) -> Option<JournalSequence> {
        read_state(&self.state)
            .entries
            .last()
            .map(JournalEntry::sequence)
    }
}

/// One bounded, immutable Transcript read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptSlice {
    from: Option<JournalSequence>,
    head: Option<JournalSequence>,
    entries: Vec<TranscriptEntry>,
}

impl TranscriptSlice {
    /// The first sequence returned by this read, if any.
    ///
    /// Keeping this value explicit lets a future repository report that the
    /// requested suffix begins after an unavailable durable range.
    #[must_use]
    pub const fn from(&self) -> Option<JournalSequence> {
        self.from
    }

    /// The newest sequence available when this read was captured.
    #[must_use]
    pub const fn head(&self) -> Option<JournalSequence> {
        self.head
    }

    #[must_use]
    pub fn entries(&self) -> &[TranscriptEntry] {
        &self.entries
    }

    #[must_use]
    pub fn into_entries(self) -> Vec<TranscriptEntry> {
        self.entries
    }
}

/// One committed semantic record displayed by Transcript projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscriptEntry {
    sequence: JournalSequence,
    record: TranscriptRecord,
}

impl TranscriptEntry {
    #[must_use]
    pub const fn sequence(&self) -> JournalSequence {
        self.sequence
    }

    #[must_use]
    pub const fn record(&self) -> &TranscriptRecord {
        &self.record
    }
}

impl From<&JournalEntry> for TranscriptEntry {
    fn from(entry: &JournalEntry) -> Self {
        let record = match entry.record() {
            SemanticRecord::CommandCommitted(command) => {
                TranscriptRecord::CommandCommitted(command.clone())
            },
            SemanticRecord::EventCommitted(event) => {
                TranscriptRecord::EventCommitted(event.clone())
            },
        };
        Self {
            sequence: entry.sequence(),
            record,
        }
    }
}

/// Frontend-independent meaning retained in the chronological Transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranscriptRecord {
    CommandCommitted(AgentCommand),
    EventCommitted(AgentEvent),
}

#[cfg(test)]
mod tests;
