use std::{error::Error, fmt, path::PathBuf};

use super::journal::{
    ConnectionCredentialAction, ConnectionOperationKind, ConnectionOperationPhase,
};

pub const MAX_OPERATION_JOURNAL_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug)]
pub enum ConnectionOperationError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    InvalidPath(PathBuf),
    UnsupportedFileType(PathBuf),
    WrongOwner(PathBuf),
    InsecurePermissions(PathBuf),
    TooLarge(PathBuf),
    Changed(PathBuf),
    InvalidContents(PathBuf),
    UnsupportedVersion {
        path: PathBuf,
        version: u32,
    },
    Conflict(PathBuf),
    OperationGuardMismatch(PathBuf),
    InvalidEntry,
    PreparedTooLarge,
    InvalidTransition {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        from: ConnectionOperationPhase,
        to: ConnectionOperationPhase,
    },
    RecoveryConflict {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        phase: ConnectionOperationPhase,
    },
    Randomness(String),
}

impl ConnectionOperationError {
    pub(super) fn io(path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for ConnectionOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::InvalidPath(path) => {
                write!(formatter, "{} has no parent directory", path.display())
            },
            Self::UnsupportedFileType(path) => write!(
                formatter,
                "{} is not a regular connection-operation journal",
                path.display()
            ),
            Self::WrongOwner(path) => write!(
                formatter,
                "{} is not owned by the current effective user",
                path.display()
            ),
            Self::InsecurePermissions(path) => write!(
                formatter,
                "{} must not grant group or other permissions",
                path.display()
            ),
            Self::TooLarge(path) => write!(
                formatter,
                "{} exceeds the {MAX_OPERATION_JOURNAL_BYTES}-byte operation-journal limit",
                path.display()
            ),
            Self::Changed(path) => write!(
                formatter,
                "{} changed while its operation intent was being read",
                path.display()
            ),
            Self::InvalidContents(path) if path.as_os_str().is_empty() => {
                formatter.write_str("the prepared connection operation is invalid")
            },
            Self::InvalidContents(path) => write!(
                formatter,
                "{} contains an invalid connection operation",
                path.display()
            ),
            Self::UnsupportedVersion { path, version } => write!(
                formatter,
                "{} uses unsupported connection-operation version {version}; expected 1",
                path.display()
            ),
            Self::Conflict(path) => write!(
                formatter,
                "{} changed outside the current serialized connection operation",
                path.display()
            ),
            Self::OperationGuardMismatch(path) => write!(
                formatter,
                "{} is outside the held connection operation lock",
                path.display()
            ),
            Self::InvalidEntry => formatter.write_str(
                "the connection operation has invalid identity, digest, action, or phase fields",
            ),
            Self::PreparedTooLarge => {
                formatter.write_str("the prepared connection operation exceeds its bounded size")
            },
            Self::InvalidTransition {
                kind,
                action,
                from,
                to,
            } => write!(
                formatter,
                "{kind:?} with {action:?} cannot advance from {from:?} to {to:?}"
            ),
            Self::RecoveryConflict {
                kind,
                action,
                phase,
            } => write!(
                formatter,
                "{kind:?} recovery at {phase:?} with {action:?} conflicts with the observed repositories; inspect current connection state before retrying"
            ),
            Self::Randomness(message) => write!(
                formatter,
                "generating a connection-operation identity failed: {message}"
            ),
        }
    }
}

impl Error for ConnectionOperationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
