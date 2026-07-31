use crate::JournalSequence;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RepositorySequence(u64);

impl RepositorySequence {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableRecordKind {
    Incremental,
    Snapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableRecord {
    kind: DurableRecordKind,
    payload: String,
    journal_cutoff: Option<JournalSequence>,
}

impl DurableRecord {
    pub fn incremental(payload: impl Into<String>) -> Self {
        Self {
            kind: DurableRecordKind::Incremental,
            payload: payload.into(),
            journal_cutoff: None,
        }
    }

    pub fn snapshot(payload: impl Into<String>) -> Self {
        Self {
            kind: DurableRecordKind::Snapshot,
            payload: payload.into(),
            journal_cutoff: None,
        }
    }

    pub const fn kind(&self) -> DurableRecordKind {
        self.kind
    }

    pub fn payload(&self) -> &str {
        &self.payload
    }

    pub(crate) const fn with_journal_cutoff(
        mut self,
        journal_cutoff: Option<JournalSequence>,
    ) -> Self {
        self.journal_cutoff = journal_cutoff;
        self
    }

    pub(crate) const fn journal_cutoff(&self) -> Option<JournalSequence> {
        self.journal_cutoff
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryEntry {
    sequence: RepositorySequence,
    record: DurableRecord,
}

impl RepositoryEntry {
    pub const fn new(sequence: RepositorySequence, record: DurableRecord) -> Self {
        Self { sequence, record }
    }

    pub const fn sequence(&self) -> RepositorySequence {
        self.sequence
    }

    pub const fn record(&self) -> &DurableRecord {
        &self.record
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppendReceipt {
    sequence: RepositorySequence,
}

impl AppendReceipt {
    pub const fn new(sequence: RepositorySequence) -> Self {
        Self { sequence }
    }

    pub const fn sequence(self) -> RepositorySequence {
        self.sequence
    }
}
