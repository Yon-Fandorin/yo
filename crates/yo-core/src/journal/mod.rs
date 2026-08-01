//! Ordered in-memory capture of committed agent semantics.

pub(crate) mod codec;
mod durable;
mod record;
mod transcript;

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub use durable::{DurabilityGapCause, JournalDurability};
use record::JournalEntry;
pub use record::JournalSequence;
pub(crate) use record::SemanticRecord;
pub use transcript::{
    TranscriptEntry, TranscriptObservation, TranscriptObservationEntry,
    TranscriptObservationSequence, TranscriptObservationSlice, TranscriptReader, TranscriptRecord,
    TranscriptSlice,
};

use crate::{AgentCommand, AgentEvent, SessionDescriptor, session_repository::SessionRepository};

#[derive(Debug)]
struct SessionJournalState {
    entries: Vec<JournalEntry>,
    observations: Vec<transcript::JournalObservationEntry>,
    durability: JournalDurability,
}

impl Default for SessionJournalState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            observations: Vec::new(),
            durability: JournalDurability::MemoryOnly,
        }
    }
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
    durable: Option<durable::DurableJournal>,
}

impl SessionJournal {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_repository_and_descriptor(
        repository: Box<dyn SessionRepository + Send>,
        descriptor: SessionDescriptor,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(SessionJournalState::default())),
            durable: Some(durable::DurableJournal::new(repository, descriptor)),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_repository(repository: Box<dyn SessionRepository + Send>) -> Self {
        let mut journal = Self::with_repository_and_descriptor(
            repository,
            crate::fixture_descriptor(crate::fixture_session(1)),
        );
        journal.initialize_durability();
        journal
    }

    pub(crate) fn initialize_durability(&mut self) {
        let Some(durable) = &mut self.durable else {
            return;
        };
        let durability = durable.initialize();
        write_state(&self.state).observe_durability(durability);
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
        let records = events
            .iter()
            .cloned()
            .map(SemanticRecord::EventCommitted)
            .collect::<Vec<_>>();
        if !records.is_empty() {
            self.append_records(records);
        }
    }

    pub(crate) fn flush_due(&mut self) {
        let Some(durable) = &mut self.durable else {
            return;
        };
        let durability = durable.flush_due();
        let mut state = write_state(&self.state);
        state.observe_durability(durability);
    }

    fn append_records(&mut self, records: Vec<SemanticRecord>) {
        let first_index = read_state(&self.state).entries.len();
        let entries = records
            .into_iter()
            .enumerate()
            .map(|(offset, record)| {
                let index = first_index
                    .checked_add(offset)
                    .expect("a Journal cannot contain more entries than usize can address");
                JournalEntry::new(JournalSequence::from_index(index), record)
            })
            .collect::<Vec<_>>();
        let durability = if let Some(durable) = &mut self.durable {
            durable.publish(&entries)
        } else {
            JournalDurability::MemoryOnly
        };
        let mut state = write_state(&self.state);
        state.observe_durability(durability);
        for entry in &entries {
            state.observe_record(entry);
        }
        state.entries.extend(entries);
    }

    #[cfg(test)]
    pub(crate) fn entries(&self) -> Vec<JournalEntry> {
        read_state(&self.state).entries.clone()
    }
}

impl SessionJournalState {
    fn observe_durability(&mut self, durability: JournalDurability) {
        if self.durability == durability {
            return;
        }
        self.durability = durability;
        self.observations
            .push(transcript::JournalObservationEntry::durability(
                transcript::TranscriptObservationSequence::from_index(self.observations.len()),
                durability,
            ));
    }

    fn observe_record(&mut self, entry: &JournalEntry) {
        self.observations
            .push(transcript::JournalObservationEntry::record(
                transcript::TranscriptObservationSequence::from_index(self.observations.len()),
                entry,
            ));
    }
}

#[cfg(test)]
mod tests;
