use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use super::{ConnectionRevision, MAX_CONNECTION_BYTES};
#[derive(Debug)]
pub enum ConnectionRepositoryError {
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
    CoordinateMismatch,
    ModelNotFound {
        provider: String,
        account: String,
        model: String,
    },
    InvalidMutation,
    PreparedTooLarge,
    OperationBusy(PathBuf),
    PendingOperation(PathBuf),
    Conflict {
        expected: ConnectionRevision,
        observed: ConnectionRevision,
    },
    Randomness(String),
    TemporaryNameRandomness(String),
    TemporaryNameCollisionExhaustion {
        attempts: usize,
    },
}

impl ConnectionRepositoryError {
    pub(super) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for ConnectionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::InvalidPath(path) => {
                write!(formatter, "{} has no parent directory", path.display())
            },
            Self::UnsupportedFileType(path) => write!(
                formatter,
                "{} is not a regular connection file",
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
                "{} exceeds the {MAX_CONNECTION_BYTES}-byte connection-file limit",
                path.display()
            ),
            Self::Changed(path) => write!(
                formatter,
                "{} changed while its connection snapshot was being read",
                path.display()
            ),
            Self::InvalidContents(path) if path.as_os_str().is_empty() => {
                formatter.write_str("the prepared connection snapshot is invalid")
            },
            Self::InvalidContents(path) => write!(
                formatter,
                "{} contains an invalid connection snapshot",
                path.display()
            ),
            Self::CoordinateMismatch => formatter.write_str(
                "the stored account and binding must name the same Provider and Account",
            ),
            Self::ModelNotFound {
                provider,
                account,
                model,
            } => write!(
                formatter,
                "stored model not found for Provider {provider}, Account {account}, Model {model}",
            ),
            Self::InvalidMutation => {
                formatter.write_str("the prepared stored connection mutation is invalid")
            },
            Self::PreparedTooLarge => {
                formatter.write_str("the prepared connection snapshot exceeds its bounded size")
            },
            Self::OperationBusy(path) => write!(
                formatter,
                "another connection operation owns {}",
                path.display()
            ),
            Self::PendingOperation(path) => write!(
                formatter,
                "{} contains a pending connection operation that this build cannot recover",
                path.display()
            ),
            Self::Conflict { expected, observed } => write!(
                formatter,
                "connection revision conflict: expected {expected}, observed {observed}; inspect the current default and retry"
            ),
            Self::Randomness(message) => write!(
                formatter,
                "generating a connection revision failed: {message}"
            ),
            Self::TemporaryNameRandomness(message) => write!(
                formatter,
                "generating a connection publication temporary name failed: {message}"
            ),
            Self::TemporaryNameCollisionExhaustion { attempts } => write!(
                formatter,
                "all {attempts} generated connection publication temporary names already exist"
            ),
        }
    }
}

impl Error for ConnectionRepositoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
