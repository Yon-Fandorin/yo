use std::{fmt, time::Instant};

mod message;

use message::MessageTracker;

use super::{JournalEntry, JournalSequence, SemanticRecord};
use crate::{
    ActivityRef, AgentEvent, SessionDescriptor,
    journal::codec::{
        JournalCommit, JournalCommitKind, JournalRecord, ReplaySequence, SequencedJournalRecord,
    },
    session_repository::{
        AppendError, DurableCutoff, RepositoryError, RepositorySequence, SessionRepository,
        StoragePressureCause,
        journal::{JournalRepository, JournalRepositoryError},
    },
};

/// Current relationship between the live Session projection and durable storage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalDurability {
    /// This Session was started without a durable repository.
    MemoryOnly,
    /// Every durable Journal record reached storage. The semantic cutoff is
    /// absent while only the initial Session descriptor is durable.
    Durable {
        journal_sequence: Option<JournalSequence>,
        repository_sequence: RepositorySequence,
    },
    /// The live Session continued after durable publication stopped.
    Gap {
        durable_cutoff: DurableCutoff,
        cause: DurabilityGapCause,
    },
}

/// Why a live Session currently has a non-durable suffix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurabilityGapCause {
    Capacity,
    Storage,
    Integrity,
}

pub(super) struct DurableJournal {
    repository: JournalRepository<Box<dyn SessionRepository + Send>>,
    records: Vec<SequencedJournalRecord>,
    messages: MessageTracker,
    started: Instant,
    live_cutoff: Option<JournalSequence>,
    status: JournalDurability,
    descriptor: Option<SessionDescriptor>,
}

impl fmt::Debug for DurableJournal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DurableJournal")
            .field("records", &self.records.len())
            .field("open_messages", &self.messages.len())
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl DurableJournal {
    pub(super) fn new(
        repository: Box<dyn SessionRepository + Send>,
        descriptor: SessionDescriptor,
    ) -> Self {
        Self {
            repository: JournalRepository::new(repository),
            records: Vec::new(),
            messages: MessageTracker::default(),
            started: Instant::now(),
            live_cutoff: None,
            status: JournalDurability::MemoryOnly,
            descriptor: Some(descriptor),
        }
    }

    /// Attempts the descriptor before any backend Session command is executed.
    ///
    /// A storage failure latches the ordinary durability gap. Later semantic work
    /// remains memory-only until a complete snapshot containing this descriptor
    /// can be published.
    pub(super) fn initialize(&mut self) -> JournalDurability {
        let Some(descriptor) = self.descriptor.take() else {
            return self.status;
        };
        self.publish_records(vec![JournalRecord::SessionDescriptor(descriptor)], None)
    }

    /// Captures semantic records and publishes any newly forced durable records.
    pub(super) fn publish(&mut self, records: &[JournalEntry]) -> JournalDurability {
        let now = self.started.elapsed();
        let mut durable = Vec::new();
        for entry in records {
            self.translate(entry.record(), now, &mut durable);
            self.live_cutoff = Some(entry.sequence());
        }
        self.publish_records(durable, self.live_cutoff)
    }

    /// Flushes message text whose oldest byte reached the one-second boundary.
    pub(super) fn flush_due(&mut self) -> JournalDurability {
        let now = self.started.elapsed();
        let durable = self.messages.flush_due(now);
        self.publish_records(durable, self.live_cutoff)
    }

    fn translate(
        &mut self,
        record: &SemanticRecord,
        now: std::time::Duration,
        durable: &mut Vec<JournalRecord>,
    ) {
        match record {
            SemanticRecord::CommandCommitted(command) => {
                self.flush_boundaries(None, durable);
                durable.push(JournalRecord::CommandCommitted(command.clone()));
            },
            SemanticRecord::EventCommitted(AgentEvent::ActivityStarted { activity, kind }) => {
                self.flush_boundaries(None, durable);
                durable.push(JournalRecord::EventCommitted(match record {
                    SemanticRecord::EventCommitted(event) => event.clone(),
                    SemanticRecord::CommandCommitted(_) => unreachable!(),
                }));
                if !self.messages.start(*activity, *kind) {
                    self.latch_integrity_gap();
                }
            },
            SemanticRecord::EventCommitted(AgentEvent::ActivityUpdated { activity, update }) => {
                let Some(segments) = self.messages.update(*activity, update, now) else {
                    // The semantic engine already rejects an update without a live Activity. Keep
                    // this defensive path volatile rather than inventing durable text ownership.
                    self.latch_integrity_gap();
                    return;
                };
                durable.extend(segments);
            },
            SemanticRecord::EventCommitted(AgentEvent::ActivityFinished { activity, outcome }) => {
                self.flush_boundaries(Some(*activity), durable);
                if let Some(terminal) = self.messages.finish(*activity, outcome) {
                    durable.push(terminal);
                } else {
                    self.latch_integrity_gap();
                }
                durable.push(JournalRecord::EventCommitted(match record {
                    SemanticRecord::EventCommitted(event) => event.clone(),
                    SemanticRecord::CommandCommitted(_) => unreachable!(),
                }));
            },
            SemanticRecord::EventCommitted(event) => {
                self.flush_boundaries(None, durable);
                durable.push(JournalRecord::EventCommitted(event.clone()));
            },
        }
    }

    fn flush_boundaries(&mut self, except: Option<ActivityRef>, durable: &mut Vec<JournalRecord>) {
        durable.extend(self.messages.flush_boundaries(except));
    }

    fn latch_integrity_gap(&mut self) {
        self.status = JournalDurability::Gap {
            durable_cutoff: durable_cutoff(self.status),
            cause: DurabilityGapCause::Integrity,
        };
    }

    fn publish_records(
        &mut self,
        records: Vec<JournalRecord>,
        journal_cutoff: Option<JournalSequence>,
    ) -> JournalDurability {
        if records.is_empty() {
            return self.status;
        }
        let first = self.records.len();
        let sequenced = records
            .into_iter()
            .enumerate()
            .map(|(offset, record)| {
                let index = first
                    .checked_add(offset)
                    .expect("a Journal cannot contain more records than usize can address");
                SequencedJournalRecord::new(ReplaySequence::new(replay_sequence(index)), record)
            })
            .collect::<Vec<_>>();
        if matches!(
            self.status,
            JournalDurability::Gap {
                cause: DurabilityGapCause::Integrity,
                ..
            }
        ) {
            // An integrity gap does not prove that another physical append is safe. Keep the
            // live model complete for diagnostics, but do not retry snapshots until a future
            // recovery owner explicitly rebuilds authority from the repository.
            self.records.extend(sequenced);
            return self.status;
        }
        let session_id = sequenced[0].record().session_id();
        let recovering_gap = matches!(self.status, JournalDurability::Gap { .. });
        if recovering_gap && !self.messages.is_empty() {
            // A complete snapshot cannot claim an open live message as recovered. Retain the
            // records as a volatile suffix and retry only after every open message has a real
            // terminal seal.
            self.records.extend(sequenced);
            return self.status;
        }
        let commit = if recovering_gap {
            let journal_cutoff = journal_cutoff
                .expect("a recovery snapshot follows at least one semantic Journal entry");
            let mut complete = self.records.clone();
            complete.extend(sequenced);
            JournalCommit::snapshot_through(journal_cutoff, complete)
        } else if journal_cutoff.is_none() {
            let descriptor = match &sequenced[0].record() {
                JournalRecord::SessionDescriptor(descriptor) if sequenced.len() == 1 => {
                    descriptor.clone()
                },
                _ => unreachable!("only descriptor initialization has no semantic cutoff"),
            };
            JournalCommit::descriptor(descriptor)
        } else {
            JournalCommit::incremental_through(journal_cutoff.expect("checked above"), sequenced)
        };

        match self.repository.append(session_id, &commit) {
            Ok(receipt) => {
                if commit.kind() == JournalCommitKind::Snapshot {
                    self.records = commit.records().to_vec();
                } else {
                    self.records.extend(commit.records().iter().cloned());
                }
                self.status = JournalDurability::Durable {
                    journal_sequence: commit.journal_cutoff(),
                    repository_sequence: receipt.sequence(),
                };
            },
            Err(error) => {
                // A failed repository read is as capable of creating a volatile suffix as a
                // failed append. Retain the complete live candidate so a later storage retry can
                // publish one authoritative snapshot instead of silently losing those records.
                if commit.kind() == JournalCommitKind::Snapshot {
                    self.records = commit.records().to_vec();
                } else {
                    self.records.extend(commit.records().iter().cloned());
                }
                self.status = gap_from(error);
            },
        }
        self.status
    }
}

fn replay_sequence(index: usize) -> u64 {
    index
        .checked_add(1)
        .and_then(|value| u64::try_from(value).ok())
        .expect("a durable Journal cannot contain more records than u64 can address")
}

fn durable_cutoff(status: JournalDurability) -> DurableCutoff {
    match status {
        JournalDurability::Durable {
            journal_sequence,
            repository_sequence,
        } => DurableCutoff::Known {
            journal_sequence,
            repository_sequence,
        },
        JournalDurability::MemoryOnly => DurableCutoff::KnownEmpty,
        JournalDurability::Gap { durable_cutoff, .. } => durable_cutoff,
    }
}

fn gap_from(error: JournalRepositoryError) -> JournalDurability {
    let (durable_cutoff, cause) = match error {
        JournalRepositoryError::Repository(RepositoryError::Unavailable { .. }) => {
            (DurableCutoff::Unknown, DurabilityGapCause::Storage)
        },
        JournalRepositoryError::Repository(RepositoryError::CorruptLog { .. })
        | JournalRepositoryError::Codec(_) => {
            (DurableCutoff::Unknown, DurabilityGapCause::Integrity)
        },
        JournalRepositoryError::Append(AppendError::StoragePressure { pressure, .. }) => (
            pressure.durable_cutoff(),
            match pressure.cause() {
                StoragePressureCause::Capacity => DurabilityGapCause::Capacity,
                StoragePressureCause::Storage => DurabilityGapCause::Storage,
            },
        ),
        JournalRepositoryError::Append(AppendError::SnapshotRequired { durable_cutoff }) => {
            (durable_cutoff, DurabilityGapCause::Integrity)
        },
        JournalRepositoryError::Append(AppendError::Repository(error)) => match error {
            RepositoryError::Unavailable { .. } => {
                (DurableCutoff::Unknown, DurabilityGapCause::Storage)
            },
            RepositoryError::CorruptLog { .. } => {
                (DurableCutoff::Unknown, DurabilityGapCause::Integrity)
            },
        },
    };
    JournalDurability::Gap {
        durable_cutoff,
        cause,
    }
}
