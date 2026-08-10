use std::num::NonZeroU64;

use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, AgentEvent, JournalSequence, SessionId,
    TurnId, TurnRef,
    journal::codec::{
        JournalCommit, JournalCommitKind, JournalRecord, SequencedJournalRecord, encode,
    },
    session_repository::{
        DurableRecord, RecordDiscovery, RepositoryEntry, RepositoryError, RepositorySequence,
        StoredSession, StoredSessionReader, StoredSessionSnapshot,
    },
};

#[derive(Debug, Default)]
pub(super) struct MemoryReader {
    pub(super) entries: Vec<RepositoryEntry>,
    pub(super) missing: bool,
}

impl StoredSessionReader for MemoryReader {
    fn discover(&self) -> Result<Vec<StoredSession>, RepositoryError> {
        Ok(Vec::new())
    }

    fn read_session(
        &self,
        _session_id: SessionId,
    ) -> Result<StoredSessionSnapshot, RepositoryError> {
        Ok(if self.missing {
            StoredSessionSnapshot::Missing
        } else {
            StoredSessionSnapshot::Present(self.entries.clone())
        })
    }

    fn read_after(
        &self,
        _session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let after = sequence.map_or(0, RepositorySequence::get);
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.sequence().get() > after)
            .take(limit)
            .cloned()
            .collect())
    }
}

pub(super) fn session() -> SessionId {
    crate::fixture_session(1)
}

pub(super) fn activity() -> ActivityRef {
    ActivityRef::new(
        TurnRef::new(session(), TurnId::new(NonZeroU64::new(2).unwrap())),
        ActivityId::new(NonZeroU64::new(3).unwrap()),
    )
}

pub(super) fn record_with_discovery(
    commit: &JournalCommit,
    discovery: RecordDiscovery,
) -> DurableRecord {
    let record = match commit.kind() {
        JournalCommitKind::Incremental => DurableRecord::incremental(encode(commit).unwrap()),
        JournalCommitKind::Snapshot => DurableRecord::snapshot(encode(commit).unwrap()),
    };
    record
        .with_journal_cutoff(commit.journal_cutoff())
        .with_discovery(discovery)
}

pub(super) fn started(sequence: u64) -> SequencedJournalRecord {
    SequencedJournalRecord::new(
        JournalSequence::new(sequence),
        JournalRecord::EventCommitted(AgentEvent::ActivityStarted {
            activity: activity(),
            kind: ActivityKind::AgentMessage,
        }),
    )
}

pub(super) fn finished(sequence: u64, outcome: ActivityOutcome) -> SequencedJournalRecord {
    SequencedJournalRecord::new(
        JournalSequence::new(sequence),
        JournalRecord::EventCommitted(AgentEvent::ActivityFinished {
            activity: activity(),
            outcome,
        }),
    )
}
