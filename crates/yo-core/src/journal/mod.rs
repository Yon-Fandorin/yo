//! Ordered in-memory capture of committed agent semantics.

pub(crate) mod codec;
mod correlation;
mod durable;
mod record;
mod transcript;

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub use durable::{DurabilityGapCause, JournalDurability};
pub use record::JournalSequence;
pub(crate) use record::{CommittedCommand, JournalEntry, SemanticRecord};
pub use transcript::{
    TranscriptEntry, TranscriptObservation, TranscriptObservationEntry,
    TranscriptObservationSequence, TranscriptObservationSlice, TranscriptReader, TranscriptRecord,
    TranscriptSlice,
};

use crate::{
    AgentCommand, AgentEvent, SessionDescriptor, SubmissionId,
    session_repository::{SessionRepository, StoredSessionContinuation},
};

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

    pub(crate) fn with_repository_and_continuation(
        repository: Box<dyn SessionRepository + Send>,
        continuation: &StoredSessionContinuation,
    ) -> Self {
        let entries = continuation.semantic_entries();
        let mut state = SessionJournalState::default();
        for record in continuation.transcript_records() {
            state
                .observations
                .push(transcript::JournalObservationEntry::record(
                    transcript::TranscriptObservationSequence::from_index(state.observations.len()),
                    record.clone(),
                ));
        }
        state.entries = entries;
        Self {
            state: Arc::new(RwLock::new(state)),
            durable: Some(durable::DurableJournal::resume(
                repository,
                continuation.descriptor().session_id(),
                continuation.snapshot(),
            )),
        }
    }

    pub(crate) fn semantic_entries(&self) -> Vec<JournalEntry> {
        read_state(&self.state).entries.clone()
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
        let committed = CommittedCommand::uncorrelated(command)
            .expect("a submission command must use append_committed_submission");
        self.append_committed(committed, events);
    }

    pub(crate) fn append_committed_submission(
        &mut self,
        command: AgentCommand,
        submission_id: SubmissionId,
        events: &[AgentEvent],
    ) {
        let committed = CommittedCommand::submission(command, submission_id)
            .expect("only a submission command may carry a SubmissionId");
        self.append_committed(committed, events);
    }

    fn append_committed(&mut self, command: CommittedCommand, events: &[AgentEvent]) {
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
        let first_sequence = read_state(&self.state).next_sequence();
        let entries = records
            .into_iter()
            .enumerate()
            .map(|(offset, record)| JournalEntry::new(first_sequence.advance_by(offset), record))
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
    fn next_sequence(&self) -> JournalSequence {
        self.entries.last().map_or_else(
            || JournalSequence::from_index(0),
            |entry| entry.sequence().advance_by(1),
        )
    }

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
        let Some(record) = TranscriptRecord::from_journal(entry) else {
            return;
        };
        self.observations
            .push(transcript::JournalObservationEntry::record(
                transcript::TranscriptObservationSequence::from_index(self.observations.len()),
                record,
            ));
    }
}

#[cfg(test)]
mod tests;
