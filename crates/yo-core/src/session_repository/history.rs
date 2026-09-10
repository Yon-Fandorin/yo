//! Validated semantic history recovered from one stored Session snapshot.

mod normalizer;
mod request_trace;
mod session_usage;

use std::{collections::HashMap, fmt, sync::Arc};

use normalizer::normalize;
pub use request_trace::{
    StoredBindingCacheState, StoredBindingCloseReason, StoredBindingTransition,
    StoredBindingTransitionMode, StoredContinuationStrategy, StoredExchangeDirection,
    StoredExchangeKind, StoredReplayExecutor, StoredRequestDetailAvailability,
    StoredRequestTraceEntry, StoredRequestTraceRecord,
};
pub use session_usage::{
    CODEX_USAGE_SCHEMA, CacheReadShare, CacheReadSummary, GROK_USAGE_SCHEMA, MANAGED_USAGE_SCHEMA,
    SessionUsage, SessionUsageAggregates, SessionUsageError, SessionUsageProjection,
    SessionUsageProvider, SessionUsageReceipt, SessionUsageSource, UsageAggregate, UsageCoverage,
    UsageValue,
};

use super::{
    RepositoryEntry, RepositoryError, RepositorySequence, StoredSessionReader,
    StoredSessionSnapshot, journal::recover_entries,
};
use crate::{
    JournalSequence, SessionDescriptor, SessionId, TranscriptRecord,
    journal::codec::{ForkHistoryCoordinate, ForkSource, JournalRecord, RecoveredJournal},
};

/// Exact captured source coordinates, independent of the last visible archival record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InheritedHistorySource {
    Empty,
    Anchor {
        record_sequence: JournalSequence,
        journal_boundary: JournalSequence,
    },
    Checkpoint {
        record_sequence: JournalSequence,
        journal_boundary: JournalSequence,
    },
    InitialFork {
        record_sequence: JournalSequence,
        journal_boundary: JournalSequence,
    },
}

/// Archival records retaining one original source Session's identities and order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InheritedHistorySection {
    source_session_id: SessionId,
    last_visible_journal_sequence: Option<JournalSequence>,
    records: Vec<TranscriptRecord>,
}

impl InheritedHistorySection {
    #[must_use]
    pub const fn source_session_id(&self) -> SessionId {
        self.source_session_id
    }

    /// Last visible semantic coordinate; this is not the exact captured source boundary.
    #[must_use]
    pub const fn last_visible_journal_sequence(&self) -> Option<JournalSequence> {
        self.last_visible_journal_sequence
    }

    #[must_use]
    pub fn records(&self) -> &[TranscriptRecord] {
        &self.records
    }
}

/// Validated child-owned archive, separate from child execution, requests, and usage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InheritedSessionHistory {
    parent_session_id: SessionId,
    source: InheritedHistorySource,
    sections: Arc<[InheritedHistorySection]>,
}

impl InheritedSessionHistory {
    #[must_use]
    pub const fn parent_session_id(&self) -> SessionId {
        self.parent_session_id
    }

    #[must_use]
    pub const fn source(&self) -> InheritedHistorySource {
        self.source
    }

    /// Original source sections in first appearance order.
    #[must_use]
    pub fn sections(&self) -> &[InheritedHistorySection] {
        &self.sections
    }
}

pub(super) fn project_inherited(
    recovered: &RecoveredJournal,
) -> Result<Option<Arc<InheritedSessionHistory>>, String> {
    let Some(seed) = recovered
        .records()
        .iter()
        .find_map(|entry| match entry.record() {
            JournalRecord::InitialForkSeed(seed) => Some(seed),
            _ => None,
        })
    else {
        return Ok(None);
    };
    let source = match seed.source() {
        ForkSource::Empty => InheritedHistorySource::Empty,
        ForkSource::Anchor(point) => InheritedHistorySource::Anchor {
            record_sequence: point.record_sequence(),
            journal_boundary: point.journal_boundary(),
        },
        ForkSource::Checkpoint(point) => InheritedHistorySource::Checkpoint {
            record_sequence: point.record_sequence(),
            journal_boundary: point.journal_boundary(),
        },
        ForkSource::InitialFork(point) => InheritedHistorySource::InitialFork {
            record_sequence: point.record_sequence(),
            journal_boundary: point.journal_boundary(),
        },
    };
    let mut source_indices = HashMap::new();
    let mut sections: Vec<InheritedHistorySection> = Vec::new();
    let mut records_by_source: Vec<Vec<&JournalRecord>> = Vec::new();
    for entry in seed.history() {
        let index = *source_indices
            .entry(entry.source_session_id())
            .or_insert_with(|| {
                let index = sections.len();
                sections.push(InheritedHistorySection {
                    source_session_id: entry.source_session_id(),
                    last_visible_journal_sequence: None,
                    records: Vec::new(),
                });
                records_by_source.push(Vec::new());
                index
            });
        if let ForkHistoryCoordinate::Journal { sequence } = entry.source_coordinate() {
            sections[index].last_visible_journal_sequence = Some(sequence);
        }
        records_by_source[index].push(entry.record().record());
    }
    for (section, records) in sections.iter_mut().zip(records_by_source) {
        section.records = normalizer::normalize_inherited(records)?;
    }
    Ok(Some(Arc::new(InheritedSessionHistory {
        parent_session_id: seed.parent_session_id(),
        source,
        sections: sections.into(),
    })))
}

pub(super) fn normalize_recovered(
    recovered: &RecoveredJournal,
) -> Result<Vec<TranscriptRecord>, String> {
    normalize(recovered)
}

/// One validated, point-in-time semantic projection of a stored Session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSessionHistory {
    descriptor: SessionDescriptor,
    journal_cutoff: Option<JournalSequence>,
    recovery: StoredSessionRecovery,
    continuity: StoredSessionContinuity,
    discovery_validation: StoredDiscoveryValidation,
    records: Vec<TranscriptRecord>,
    request_trace: Vec<StoredRequestTraceEntry>,
    inherited_history: Option<Arc<InheritedSessionHistory>>,
}

impl StoredSessionHistory {
    #[must_use]
    pub const fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn journal_cutoff(&self) -> Option<JournalSequence> {
        self.journal_cutoff
    }

    #[must_use]
    pub const fn recovery(&self) -> StoredSessionRecovery {
        self.recovery
    }

    /// Whether the physical history can prove that no volatile suffix was lost.
    #[must_use]
    pub const fn continuity(&self) -> StoredSessionContinuity {
        self.continuity
    }

    /// Result of validating every physical discovery summary against semantic Journal authority.
    #[must_use]
    pub const fn discovery_validation(&self) -> StoredDiscoveryValidation {
        self.discovery_validation
    }

    #[must_use]
    pub const fn discovery_consistent(&self) -> bool {
        matches!(
            self.discovery_validation,
            StoredDiscoveryValidation::Consistent
        )
    }

    /// Returns the recovered frontend-independent records in durable order.
    #[must_use]
    pub fn records(&self) -> &[TranscriptRecord] {
        &self.records
    }

    /// Source-qualified archival history, excluded from this Session's records and usage.
    #[must_use]
    pub fn inherited_history(&self) -> Option<&InheritedSessionHistory> {
        self.inherited_history.as_deref()
    }

    /// Returns every payload-free Request correlation fact in durable Journal order.
    #[must_use]
    pub fn request_trace(&self) -> &[StoredRequestTraceEntry] {
        &self.request_trace
    }

    pub fn session_usage(&self) -> Result<SessionUsageProjection, SessionUsageError> {
        SessionUsageProjection::from_records(&self.records)
    }
}

/// Whether archival recovery had to close a message whose durable terminal was absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredSessionRecovery {
    NotRequired,
    Interrupted,
}

/// What a stored physical history can prove about process-local semantic suffixes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredSessionContinuity {
    /// The current v1 format cannot prove whether a stopped writer lost volatile records.
    NotObservable,
}

/// Result of validating the physical discovery summaries in one stored Session history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredDiscoveryValidation {
    Consistent,
    Mismatch(StoredDiscoveryMismatch),
}

/// First physical discovery summary that disagrees with semantic Journal authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredDiscoveryMismatch {
    repository_sequence: RepositorySequence,
    kind: StoredDiscoveryMismatchKind,
}

impl StoredDiscoveryMismatch {
    #[must_use]
    pub const fn new(
        repository_sequence: RepositorySequence,
        kind: StoredDiscoveryMismatchKind,
    ) -> Self {
        Self {
            repository_sequence,
            kind,
        }
    }

    #[must_use]
    pub const fn repository_sequence(self) -> RepositorySequence {
        self.repository_sequence
    }

    #[must_use]
    pub const fn kind(self) -> StoredDiscoveryMismatchKind {
        self.kind
    }
}

impl fmt::Display for StoredDiscoveryMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let repository_sequence = self.repository_sequence.get();
        match self.kind {
            StoredDiscoveryMismatchKind::Missing => write!(
                formatter,
                "metadata is missing at repository sequence {repository_sequence}"
            ),
            StoredDiscoveryMismatchKind::Descriptor => write!(
                formatter,
                "descriptor disagrees with its semantic Journal at repository sequence {repository_sequence}"
            ),
            StoredDiscoveryMismatchKind::BindingEpoch { claimed } => write!(
                formatter,
                "binding epoch {claimed} at repository sequence {repository_sequence} has no semantic Journal binding evidence"
            ),
            StoredDiscoveryMismatchKind::ContinuationAnchor { referenced } => write!(
                formatter,
                "Continuation Anchor Journal sequence {} at repository sequence {repository_sequence} has no semantic Journal anchor evidence",
                referenced.get()
            ),
            StoredDiscoveryMismatchKind::InitialForkSeed { referenced } => write!(
                formatter,
                "Initial fork seed Journal sequence {} at repository sequence {repository_sequence} has no semantic Journal seed evidence",
                referenced.get()
            ),
            StoredDiscoveryMismatchKind::MissingInitialForkSeed { expected } => write!(
                formatter,
                "Initial fork seed Journal sequence {} is missing at repository sequence {repository_sequence}",
                expected.get()
            ),
            StoredDiscoveryMismatchKind::BindingEpochDisagreement { expected, claimed } => write!(
                formatter,
                "binding epoch {claimed} disagrees with semantic Journal epoch {expected} at repository sequence {repository_sequence}"
            ),
            StoredDiscoveryMismatchKind::ContinuationAnchorDisagreement { expected, claimed } => {
                write!(
                    formatter,
                    "Continuation Anchor Journal sequence {} disagrees with semantic Journal sequence {} at repository sequence {repository_sequence}",
                    claimed.get(),
                    expected.get()
                )
            },
            StoredDiscoveryMismatchKind::MissingBindingEpoch { expected } => write!(
                formatter,
                "binding epoch {expected} is missing at repository sequence {repository_sequence}"
            ),
            StoredDiscoveryMismatchKind::MissingContinuationAnchor { expected } => write!(
                formatter,
                "Continuation Anchor Journal sequence {} is missing at repository sequence {repository_sequence}",
                expected.get()
            ),
        }
    }
}

/// Reason a physical discovery summary cannot be derived from its semantic Journal prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredDiscoveryMismatchKind {
    Missing,
    Descriptor,
    BindingEpoch {
        claimed: u64,
    },
    ContinuationAnchor {
        referenced: JournalSequence,
    },
    /// The physical hint has no validated initial child seed behind it.
    InitialForkSeed {
        referenced: JournalSequence,
    },
    /// A validated executable initial seed is omitted from physical discovery.
    MissingInitialForkSeed {
        expected: JournalSequence,
    },
    BindingEpochDisagreement {
        expected: u64,
        claimed: u64,
    },
    ContinuationAnchorDisagreement {
        expected: JournalSequence,
        claimed: JournalSequence,
    },
    MissingBindingEpoch {
        expected: u64,
    },
    MissingContinuationAnchor {
        expected: JournalSequence,
    },
}

/// Failure to validate and recover one stored Session's semantic Journal.
#[derive(Debug)]
pub enum StoredSessionReadError {
    NotFound { session_id: SessionId },
    Incomplete { session_id: SessionId },
    Repository(RepositoryError),
    Invalid { detail: String },
}

impl fmt::Display for StoredSessionReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { session_id } => {
                write!(formatter, "stored Session {session_id} was not found")
            },
            Self::Incomplete { session_id } => {
                write!(
                    formatter,
                    "stored Session {session_id} has no complete envelope"
                )
            },
            Self::Repository(error) => error.fmt(formatter),
            Self::Invalid { detail } => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for StoredSessionReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            Self::NotFound { .. } | Self::Incomplete { .. } | Self::Invalid { .. } => None,
        }
    }
}

/// Recovers one durable Session without acquiring a writer lease or exposing physical records.
pub fn read_stored_session(
    reader: &(impl StoredSessionReader + ?Sized),
    session_id: SessionId,
) -> Result<StoredSessionHistory, StoredSessionReadError> {
    let entries = match reader
        .read_session(session_id)
        .map_err(StoredSessionReadError::Repository)?
    {
        StoredSessionSnapshot::Missing => {
            return Err(StoredSessionReadError::NotFound { session_id });
        },
        StoredSessionSnapshot::Present(entries) if entries.is_empty() => {
            return Err(StoredSessionReadError::Incomplete { session_id });
        },
        StoredSessionSnapshot::Present(entries) => entries,
    };
    let recovered =
        recover_entries(session_id, &entries).map_err(|error| invalid_stored(error.to_string()))?;
    let descriptor = recovered
        .descriptor()
        .cloned()
        .ok_or_else(|| invalid_stored(format!("stored Session {session_id} has no descriptor")))?;
    let discovery_validation = validate_discovery(&entries, &descriptor, &recovered);
    let recovery = if recovered.recovery_commit().is_some() {
        StoredSessionRecovery::Interrupted
    } else {
        StoredSessionRecovery::NotRequired
    };
    let records = normalize(&recovered).map_err(invalid_stored)?;
    let request_trace = request_trace::project(&recovered);
    let inherited_history = project_inherited(&recovered).map_err(invalid_stored)?;
    Ok(StoredSessionHistory {
        descriptor,
        journal_cutoff: recovered.journal_cutoff(),
        recovery,
        continuity: StoredSessionContinuity::NotObservable,
        discovery_validation,
        records,
        request_trace,
        inherited_history,
    })
}

pub(super) fn validate_discovery(
    entries: &[RepositoryEntry],
    descriptor: &SessionDescriptor,
    recovered: &RecoveredJournal,
) -> StoredDiscoveryValidation {
    for (entry, expected) in entries.iter().zip(recovered.discovery_states()) {
        let repository_sequence = entry.sequence();
        let Some(discovery) = entry.record().discovery() else {
            return StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch::new(
                repository_sequence,
                StoredDiscoveryMismatchKind::Missing,
            ));
        };
        let kind = if discovery.descriptor() != descriptor {
            Some(StoredDiscoveryMismatchKind::Descriptor)
        } else if expected.initial_fork_seed() != discovery.initial_fork_seed() {
            match discovery.initial_fork_seed() {
                Some(referenced) => {
                    Some(StoredDiscoveryMismatchKind::InitialForkSeed { referenced })
                },
                None => Some(StoredDiscoveryMismatchKind::MissingInitialForkSeed {
                    expected: expected
                        .initial_fork_seed()
                        .expect("different hints have an expected seed"),
                }),
            }
        } else {
            discovery_coordinates_mismatch(
                expected.binding_epoch(),
                discovery.binding_epoch(),
                expected.continuation_anchor(),
                discovery.continuation_anchor(),
            )
        };
        if let Some(kind) = kind {
            return StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch::new(
                repository_sequence,
                kind,
            ));
        }
    }
    StoredDiscoveryValidation::Consistent
}

fn discovery_coordinates_mismatch(
    expected_epoch: Option<u64>,
    claimed_epoch: Option<u64>,
    expected_anchor: Option<JournalSequence>,
    claimed_anchor: Option<JournalSequence>,
) -> Option<StoredDiscoveryMismatchKind> {
    if expected_epoch != claimed_epoch {
        return Some(match (expected_epoch, claimed_epoch) {
            (None, Some(claimed)) => StoredDiscoveryMismatchKind::BindingEpoch { claimed },
            (Some(expected), None) => StoredDiscoveryMismatchKind::MissingBindingEpoch { expected },
            (Some(expected), Some(claimed)) => {
                StoredDiscoveryMismatchKind::BindingEpochDisagreement { expected, claimed }
            },
            (None, None) => unreachable!("equal optional epochs were handled above"),
        });
    }
    if expected_anchor != claimed_anchor {
        return Some(match (expected_anchor, claimed_anchor) {
            (None, Some(referenced)) => {
                StoredDiscoveryMismatchKind::ContinuationAnchor { referenced }
            },
            (Some(expected), None) => {
                StoredDiscoveryMismatchKind::MissingContinuationAnchor { expected }
            },
            (Some(expected), Some(claimed)) => {
                StoredDiscoveryMismatchKind::ContinuationAnchorDisagreement { expected, claimed }
            },
            (None, None) => unreachable!("equal optional Anchors were handled above"),
        });
    }
    None
}

fn invalid_stored(detail: impl Into<String>) -> StoredSessionReadError {
    StoredSessionReadError::Invalid {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
