use crate::{AgentCommand, AgentEvent};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct JournalSequence(u64);

impl JournalSequence {
    pub(super) fn from_index(index: usize) -> Self {
        let value = index
            .checked_add(1)
            .and_then(|value| u64::try_from(value).ok())
            .expect("a Journal cannot contain more entries than u64 can address");
        Self(value)
    }

    #[cfg(test)]
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SemanticRecord {
    CommandCommitted(AgentCommand),
    EventCommitted(AgentEvent),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JournalEntry {
    sequence: JournalSequence,
    record: SemanticRecord,
}

impl JournalEntry {
    pub(super) const fn new(sequence: JournalSequence, record: SemanticRecord) -> Self {
        Self { sequence, record }
    }

    #[cfg(test)]
    pub(crate) const fn sequence(&self) -> JournalSequence {
        self.sequence
    }

    #[cfg(test)]
    pub(crate) const fn record(&self) -> &SemanticRecord {
        &self.record
    }
}
