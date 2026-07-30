//! Storage-neutral durable Session records.

mod local;
mod record;

use std::fmt;

pub use local::LocalSessionRepository;
pub use record::{
    AppendReceipt, DurableRecord, DurableRecordKind, RepositoryEntry, RepositorySequence,
};

use crate::SessionId;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoragePressureCause {
    Capacity,
    Storage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableCutoff {
    Unknown,
    Known(Option<RepositorySequence>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoragePressure {
    durable_cutoff: DurableCutoff,
    cause: StoragePressureCause,
}

impl StoragePressure {
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
        durable_cutoff: Option<RepositorySequence>,
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
    CorruptLog { line: usize, reason: String },
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { message } => formatter.write_str(message),
            Self::CorruptLog { line, reason } => {
                write!(formatter, "corrupt Session log at line {line}: {reason}")
            },
        }
    }
}

impl std::error::Error for RepositoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unavailable { .. } | Self::CorruptLog { .. } => None,
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
