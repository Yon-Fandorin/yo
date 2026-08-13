use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use super::storage::MAX_CREDENTIAL_FILE_BYTES;

#[derive(Debug)]
pub enum LocalCredentialStoreError {
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
    PreparedTooLarge,
    InvalidMutation,
    Conflict(PathBuf),
    Randomness(String),
}

impl LocalCredentialStoreError {
    pub(super) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for LocalCredentialStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::InvalidPath(path) => {
                write!(formatter, "{} has no parent directory", path.display())
            },
            Self::UnsupportedFileType(path) => {
                write!(
                    formatter,
                    "{} is not a regular credential file",
                    path.display()
                )
            },
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
                "{} exceeds the {MAX_CREDENTIAL_FILE_BYTES}-byte credential-file limit",
                path.display()
            ),
            Self::Changed(path) => write!(
                formatter,
                "{} changed while its credential snapshot was being read",
                path.display()
            ),
            Self::InvalidContents(path) if path.as_os_str().is_empty() => {
                formatter.write_str("the prepared credential snapshot is invalid")
            },
            Self::InvalidContents(path) => {
                write!(formatter, "{} contains invalid credentials", path.display())
            },
            Self::UnsupportedVersion { path, version } => write!(
                formatter,
                "{} uses unsupported credential version {version}; expected 1",
                path.display()
            ),
            Self::PreparedTooLarge => {
                formatter.write_str("the prepared credential snapshot exceeds its bounded size")
            },
            Self::InvalidMutation => formatter.write_str("invalid prepared credential mutation"),
            Self::Conflict(path) => write!(
                formatter,
                "{} changed after the credential mutation was prepared; retry from a fresh snapshot",
                path.display()
            ),
            Self::Randomness(message) => {
                write!(
                    formatter,
                    "generating a private credential receipt failed: {message}"
                )
            },
        }
    }
}

impl Error for LocalCredentialStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
