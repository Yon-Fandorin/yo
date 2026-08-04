//! Validated semantic history recovered from one stored Session snapshot.

mod normalizer;

use std::fmt;

use normalizer::normalize;

use super::{
    RepositoryEntry, RepositoryError, RepositorySequence, StoredSessionReader,
    StoredSessionSnapshot, journal::recover_entries,
};
use crate::{JournalSequence, SessionDescriptor, SessionId, TranscriptRecord};

/// One validated, point-in-time semantic projection of a stored Session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSessionHistory {
    descriptor: SessionDescriptor,
    journal_cutoff: Option<JournalSequence>,
    recovery: StoredSessionRecovery,
    continuity: StoredSessionContinuity,
    discovery_validation: StoredDiscoveryValidation,
    records: Vec<TranscriptRecord>,
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
        }
    }
}

/// Reason a physical discovery summary cannot be derived from its semantic Journal prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredDiscoveryMismatchKind {
    Missing,
    Descriptor,
    BindingEpoch { claimed: u64 },
    ContinuationAnchor { referenced: JournalSequence },
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
    let discovery_validation = validate_discovery(&entries, &descriptor);
    let recovery = if recovered.recovery_commit().is_some() {
        StoredSessionRecovery::Interrupted
    } else {
        StoredSessionRecovery::NotRequired
    };
    let records = normalize(&recovered).map_err(invalid_stored)?;
    Ok(StoredSessionHistory {
        descriptor,
        journal_cutoff: recovered.journal_cutoff(),
        recovery,
        continuity: StoredSessionContinuity::NotObservable,
        discovery_validation,
        records,
    })
}

fn validate_discovery(
    entries: &[RepositoryEntry],
    descriptor: &SessionDescriptor,
) -> StoredDiscoveryValidation {
    for entry in entries {
        let repository_sequence = entry.sequence();
        let Some(discovery) = entry.record().discovery() else {
            return StoredDiscoveryValidation::Mismatch(StoredDiscoveryMismatch::new(
                repository_sequence,
                StoredDiscoveryMismatchKind::Missing,
            ));
        };
        let kind = if discovery.descriptor() != descriptor {
            Some(StoredDiscoveryMismatchKind::Descriptor)
        } else if let Some(claimed) = discovery.binding_epoch() {
            // Semantic v1 has no binding record from which this value can be derived.
            Some(StoredDiscoveryMismatchKind::BindingEpoch { claimed })
        } else {
            // Semantic v1 likewise has no accepted outcome, binding, or locator records from
            // which a complete Continuation Anchor can be validated.
            discovery
                .continuation_anchor()
                .map(|referenced| StoredDiscoveryMismatchKind::ContinuationAnchor { referenced })
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

fn invalid_stored(detail: impl Into<String>) -> StoredSessionReadError {
    StoredSessionReadError::Invalid {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
