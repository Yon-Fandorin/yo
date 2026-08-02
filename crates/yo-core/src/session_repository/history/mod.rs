//! Validated semantic history recovered from one stored Session snapshot.

mod normalizer;

use std::fmt;

use normalizer::normalize;

use super::{
    RepositoryError, StoredSessionReader, StoredSessionSnapshot, journal::recover_entries,
};
use crate::{JournalSequence, SessionDescriptor, SessionId, TranscriptRecord};

/// One validated, point-in-time semantic projection of a stored Session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSessionHistory {
    descriptor: SessionDescriptor,
    journal_cutoff: Option<JournalSequence>,
    recovery: StoredSessionRecovery,
    continuity: StoredSessionContinuity,
    discovery_consistent: bool,
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

    #[must_use]
    pub const fn discovery_consistent(&self) -> bool {
        self.discovery_consistent
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
    let discovery = entries
        .iter()
        .map(|entry| entry.record().discovery().cloned())
        .collect::<Vec<_>>();
    let recovered =
        recover_entries(session_id, &entries).map_err(|error| invalid_stored(error.to_string()))?;
    let descriptor = recovered
        .descriptor()
        .cloned()
        .ok_or_else(|| invalid_stored(format!("stored Session {session_id} has no descriptor")))?;
    let discovery_consistent = discovery.iter().all(|candidate| {
        candidate
            .as_ref()
            .is_some_and(|candidate| candidate.descriptor() == &descriptor)
    });
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
        discovery_consistent,
        records,
    })
}

fn invalid_stored(detail: impl Into<String>) -> StoredSessionReadError {
    StoredSessionReadError::Invalid {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests;
