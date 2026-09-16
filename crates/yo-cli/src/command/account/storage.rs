use std::{
    error::Error,
    fmt, io,
    path::{Path, PathBuf},
};

use crate::command::account::domain::AccountCapacityReport;

mod codec;
mod filesystem;
#[cfg(test)]
mod tests;

const SCHEMA: &str = "yo.account-capacity-cache/v1alpha2";
const LEGACY_SCHEMA: &str = "yo.account-capacity-cache/v1alpha1";
const MAX_CACHE_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const FILE_MODE: u32 = 0o600;
const DIRECTORY_MODE: u32 = 0o700;
const LOCK_FILE: &str = ".account-capacity.lock";

#[derive(Debug)]
pub(super) enum StorageError {
    Io { path: PathBuf, source: io::Error },
    InvalidPath(PathBuf),
    UnsupportedFileType(PathBuf),
    WrongOwner(PathBuf),
    InsecurePermissions(PathBuf),
    TooLarge(PathBuf),
    Changed(PathBuf),
    InvalidContents(PathBuf),
    Randomness(String),
}

impl StorageError {
    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::InvalidPath(path) => {
                write!(formatter, "{} has no parent directory", path.display())
            },
            Self::UnsupportedFileType(path) => write!(
                formatter,
                "{} is not a regular account-capacity cache file",
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
                "{} exceeds the {MAX_CACHE_BYTES}-byte account-capacity cache limit",
                path.display()
            ),
            Self::Changed(path) => write!(
                formatter,
                "{} changed while its account-capacity cache was being read",
                path.display()
            ),
            Self::InvalidContents(path) if path.as_os_str().is_empty() => {
                formatter.write_str("the account-capacity cache contains invalid contents")
            },
            Self::InvalidContents(path) => write!(
                formatter,
                "{} contains an invalid account-capacity cache",
                path.display()
            ),
            Self::Randomness(message) => {
                write!(
                    formatter,
                    "generating an account-capacity cache name failed: {message}"
                )
            },
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(super) fn load(path: &Path) -> Result<Vec<AccountCapacityReport>, StorageError> {
    let Some(encoded) = filesystem::read_bytes(path)? else {
        return Ok(Vec::new());
    };
    codec::decode(path, &encoded)
}

pub(super) fn upsert(path: &Path, updates: &[AccountCapacityReport]) -> Result<(), StorageError> {
    if updates.is_empty() {
        return Ok(());
    }
    filesystem::reject_symlink(path)?;
    let (parent, lock) = filesystem::lock_repository(path)?;
    let result = (|| {
        let mut reports = load(path)?;
        for update in updates {
            let coordinate = (
                update.snapshot().provider().as_str(),
                update.snapshot().account().as_str(),
            );
            if matches!(coordinate.0, "codex" | "grok") {
                reports.retain(|existing| {
                    existing.snapshot().provider().as_str() != coordinate.0
                        || existing.snapshot().account().as_str() == coordinate.1
                });
            }
            if let Some(existing) = reports.iter_mut().find(|existing| {
                existing.snapshot().provider().as_str() == coordinate.0
                    && existing.snapshot().account().as_str() == coordinate.1
            }) {
                *existing = update.clone();
            } else {
                reports.push(update.clone());
            }
        }
        reports.sort_by(|left, right| {
            left.snapshot()
                .provider()
                .as_str()
                .cmp(right.snapshot().provider().as_str())
                .then_with(|| {
                    left.snapshot()
                        .account()
                        .as_str()
                        .cmp(right.snapshot().account().as_str())
                })
        });
        let encoded = codec::encode(&reports)?;
        filesystem::publish(path, &parent, &encoded)
    })();
    drop(lock);
    result
}
