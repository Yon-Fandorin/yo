//! Ordered in-memory capture of committed agent semantics.

mod record;

pub(crate) use record::SemanticRecord;
use record::{JournalEntry, JournalSequence};

use crate::{AgentCommand, AgentEvent};

#[derive(Debug, Default)]
pub(crate) struct SessionJournal {
    entries: Vec<JournalEntry>,
}

impl SessionJournal {
    pub(crate) fn new() -> Self {
        Self::default()
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
        let first_index = self.entries.len();
        self.entries
            .extend(records.into_iter().enumerate().map(|(offset, record)| {
                let index = first_index
                    .checked_add(offset)
                    .expect("a Journal cannot contain more entries than usize can address");
                let sequence = JournalSequence::from_index(index);
                JournalEntry::new(sequence, record)
            }));
    }

    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests;
