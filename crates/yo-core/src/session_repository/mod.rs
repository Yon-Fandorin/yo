//! Storage-neutral durable Session records.

mod history;
pub(crate) mod journal;
mod local;
mod record;

use std::fmt;

pub use history::{
    StoredSessionContinuity, StoredSessionHistory, StoredSessionReadError, StoredSessionRecovery,
    read_stored_session,
};
pub use local::{LocalSessionReader, LocalSessionRepository};
pub use record::{
    AppendReceipt, ContinuationEligibility, DurableRecord, DurableRecordKind, RecordDiscovery,
    RepositoryEntry, RepositorySequence, SessionDiscovery, SessionRecordVersion, StoredSession,
    StoredSessionSummary, StoredSessionUnavailableReason,
};

use crate::{JournalSequence, SessionId};

pub trait SessionRepository {
    fn append(
        &mut self,
        session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError>;

    fn read_after(
        &self,
        session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError>;
}

/// Read-only access to durable Sessions without executable continuation.
pub trait StoredSessionReader {
    fn discover(&self) -> Result<Vec<StoredSession>, RepositoryError>;

    /// Captures all committed physical records for one point-in-time Session view.
    fn read_session(&self, session_id: SessionId)
    -> Result<StoredSessionSnapshot, RepositoryError>;

    fn read_after(
        &self,
        session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError>;
}

/// One presence-aware, point-in-time capture of a Session's physical records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoredSessionSnapshot {
    Missing,
    Present(Vec<RepositoryEntry>),
}

impl<R> StoredSessionReader for Box<R>
where
    R: StoredSessionReader + ?Sized,
{
    fn discover(&self) -> Result<Vec<StoredSession>, RepositoryError> {
        (**self).discover()
    }

    fn read_session(
        &self,
        session_id: SessionId,
    ) -> Result<StoredSessionSnapshot, RepositoryError> {
        (**self).read_session(session_id)
    }

    fn read_after(
        &self,
        session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        (**self).read_after(session_id, sequence, limit)
    }
}

impl<R> SessionRepository for Box<R>
where
    R: SessionRepository + ?Sized,
{
    fn append(
        &mut self,
        session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        (**self).append(session_id, record)
    }

    fn read_after(
        &self,
        session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        (**self).read_after(session_id, sequence, limit)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoragePressureCause {
    Capacity,
    Storage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableCutoff {
    Unknown,
    KnownEmpty,
    Known {
        journal_sequence: Option<JournalSequence>,
        repository_sequence: RepositorySequence,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoragePressure {
    durable_cutoff: DurableCutoff,
    cause: StoragePressureCause,
}

impl StoragePressure {
    pub const fn new(durable_cutoff: DurableCutoff, cause: StoragePressureCause) -> Self {
        Self {
            durable_cutoff,
            cause,
        }
    }

    pub const fn durable_cutoff(&self) -> DurableCutoff {
        self.durable_cutoff
    }

    pub const fn cause(&self) -> StoragePressureCause {
        self.cause
    }
}

#[derive(Debug)]
pub enum AppendError {
    SnapshotRequired {
        durable_cutoff: DurableCutoff,
    },
    StoragePressure {
        pressure: StoragePressure,
        source: Option<RepositoryError>,
    },
    Repository(RepositoryError),
}

impl AppendError {
    pub const fn storage_pressure(&self) -> Option<&StoragePressure> {
        match self {
            Self::StoragePressure { pressure, .. } => Some(pressure),
            Self::SnapshotRequired { .. } | Self::Repository(_) => None,
        }
    }
}

impl fmt::Display for AppendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SnapshotRequired { durable_cutoff } => {
                write!(
                    formatter,
                    "a complete Session snapshot is required after durable cutoff {durable_cutoff:?}"
                )
            },
            Self::StoragePressure { pressure, .. } => {
                write!(
                    formatter,
                    "durable append stopped at {:?} because of {:?} storage pressure",
                    pressure.durable_cutoff, pressure.cause
                )
            },
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AppendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StoragePressure {
                source: Some(source),
                ..
            } => Some(source),
            Self::Repository(error) => Some(error),
            Self::SnapshotRequired { .. } | Self::StoragePressure { source: None, .. } => None,
        }
    }
}

#[derive(Debug)]
pub enum RepositoryError {
    Unavailable { message: String },
    Quarantined { message: String },
    UnsupportedSchema { schema: String },
    CorruptLog { line: usize, reason: String },
    CorruptTail { reason: String },
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { message } => formatter.write_str(message),
            Self::Quarantined { message } => formatter.write_str(message),
            Self::UnsupportedSchema { schema } => {
                write!(formatter, "unsupported Session record schema {schema:?}")
            },
            Self::CorruptLog { line, reason } => {
                write!(formatter, "corrupt Session log at line {line}: {reason}")
            },
            Self::CorruptTail { reason } => {
                write!(formatter, "corrupt Session log at tail envelope: {reason}")
            },
        }
    }
}

impl std::error::Error for RepositoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable { .. }
            | Self::Quarantined { .. }
            | Self::UnsupportedSchema { .. }
            | Self::CorruptLog { .. }
            | Self::CorruptTail { .. } => None,
        }
    }
}

impl From<std::io::Error> for RepositoryError {
    fn from(error: std::io::Error) -> Self {
        Self::Unavailable {
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
