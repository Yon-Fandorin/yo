//! Ordered in-memory capture of committed agent semantics.

pub(crate) mod codec;
mod record;
mod transcript;

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use record::JournalEntry;
pub use record::JournalSequence;
pub(crate) use record::SemanticRecord;
pub use transcript::{TranscriptEntry, TranscriptReader, TranscriptRecord, TranscriptSlice};

use crate::{AgentCommand, AgentEvent};

#[derive(Debug, Default)]
struct SessionJournalState {
    entries: Vec<JournalEntry>,
}

fn read_state(state: &RwLock<SessionJournalState>) -> RwLockReadGuard<'_, SessionJournalState> {
    // Append-only entries remain a valid contiguous prefix if a panic poisons the lock.
    state.read().unwrap_or_else(|error| error.into_inner())
}

fn write_state(state: &RwLock<SessionJournalState>) -> RwLockWriteGuard<'_, SessionJournalState> {
    // Recover the same valid prefix so later appends can continue its sequence.
    state.write().unwrap_or_else(|error| error.into_inner())
}

#[derive(Debug, Default)]
pub(crate) struct SessionJournal {
    state: Arc<RwLock<SessionJournalState>>,
}

impl SessionJournal {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn transcript_reader(&self) -> TranscriptReader {
        TranscriptReader {
            state: Arc::clone(&self.state),
        }
    }

    pub(crate) fn append_committed_command(
        &mut self,
        command: AgentCommand,
        events: &[AgentEvent],
    ) {
        let mut records = Vec::with_capacity(events.len() + 1);
        records.push(SemanticRecord::CommandCommitted(command));
        records.extend(events.iter().cloned().map(SemanticRecord::EventCommitted));
        self.append_records(records);
    }

    pub(crate) fn append_events(&mut self, events: &[AgentEvent]) {
        self.append_records(
            events
                .iter()
                .cloned()
                .map(SemanticRecord::EventCommitted)
                .collect(),
        );
    }

    fn append_records(&mut self, records: Vec<SemanticRecord>) {
        let mut state = write_state(&self.state);
        let first_index = state.entries.len();
        state
            .entries
            .extend(records.into_iter().enumerate().map(|(offset, record)| {
                let index = first_index
                    .checked_add(offset)
                    .expect("a Journal cannot contain more entries than usize can address");
                let sequence = JournalSequence::from_index(index);
                JournalEntry::new(sequence, record)
            }));
    }

    #[cfg(test)]
    pub(crate) fn entries(&self) -> Vec<JournalEntry> {
        read_state(&self.state).entries.clone()
    }
}

#[cfg(test)]
mod tests;
