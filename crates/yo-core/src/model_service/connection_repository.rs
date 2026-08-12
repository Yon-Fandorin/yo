use std::{
    fmt, fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::StartupTarget;

mod error;
mod wire;

pub use error::ConnectionRepositoryError;

const MAX_CONNECTION_BYTES: u64 = 1024 * 1024;
const FILE_MODE: u32 = 0o600;
const DIRECTORY_MODE: u32 = 0o700;
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;
const FILE_TYPE_MASK: u32 = libc::S_IFMT;
const REPOSITORY_LOCK_FILE: &str = ".connections.lock";
const OPERATION_LOCK_FILE: &str = ".connection-operation.lock";
const PENDING_OPERATION_FILE: &str = "connection-operation.yaml";

/// Opaque compare-and-swap token for one complete public connection snapshot.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ConnectionRevision {
    Absent,
    Token(String),
}

impl ConnectionRevision {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

impl fmt::Display for ConnectionRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => formatter.write_str("absent"),
            Self::Token(token) => formatter.write_str(token),
        }
    }
}

/// One bounded immutable public repository capture.
#[derive(Clone, Debug)]
pub struct ConnectionSnapshot {
    revision: ConnectionRevision,
    preference: Option<StartupTarget>,
    encoded: Vec<u8>,
}

impl ConnectionSnapshot {
    #[must_use]
    pub const fn revision(&self) -> &ConnectionRevision {
        &self.revision
    }

    #[must_use]
    pub const fn preference(&self) -> Option<&StartupTarget> {
        self.preference.as_ref()
    }

    /// Prepares exact old-or-new bytes without changing the repository.
    pub fn prepare_preference(
        &self,
        preference: Option<StartupTarget>,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        if self.preference == preference {
            return Ok(None);
        }
        let planned_revision = wire::new_revision()?;
        let planned_bytes = wire::encode(&planned_revision, preference.as_ref())?;
        if planned_bytes.len() as u64 > MAX_CONNECTION_BYTES {
            return Err(ConnectionRepositoryError::PreparedTooLarge);
        }
        Ok(Some(PreparedConnectionMutation {
            expected_revision: self.revision.clone(),
            planned_revision,
            planned_bytes,
            preference,
        }))
    }
}

/// One immutable exact public mutation prepared from a captured revision.
#[derive(Clone, Debug)]
pub struct PreparedConnectionMutation {
    expected_revision: ConnectionRevision,
    planned_revision: ConnectionRevision,
    planned_bytes: Vec<u8>,
    preference: Option<StartupTarget>,
}

impl PreparedConnectionMutation {
    #[must_use]
    pub const fn expected_revision(&self) -> &ConnectionRevision {
        &self.expected_revision
    }

    #[must_use]
    pub const fn planned_revision(&self) -> &ConnectionRevision {
        &self.planned_revision
    }

    #[must_use]
    pub const fn preference(&self) -> Option<&StartupTarget> {
        self.preference.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionCommit {
    Committed,
    AlreadyCommitted,
}

/// Storage-neutral preference publication boundary used by connection orchestration.
pub trait ConnectionRepository {
    type OperationGuard;

    fn acquire_operation(&self) -> Result<Self::OperationGuard, ConnectionRepositoryError>;
    fn recover_pending_operation(&self) -> Result<(), ConnectionRepositoryError>;
    fn capture(&self) -> Result<ConnectionSnapshot, ConnectionRepositoryError>;
    fn commit(
        &self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionRepositoryError>;
}

/// Local bounded `connections.yaml` repository with exact revision CAS.
#[derive(Clone, Debug)]
pub struct LocalConnectionRepository {
    path: PathBuf,
}

impl LocalConnectionRepository {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Captures without creating the file or its parent directory.
    pub fn capture(&self) -> Result<ConnectionSnapshot, ConnectionRepositoryError> {
        read_snapshot(&self.path)
    }

    /// Acquires the process-wide operation lane shared by connection mutations.
    pub fn acquire_operation(
        &self,
    ) -> Result<LocalConnectionOperationGuard, ConnectionRepositoryError> {
        let parent = prepare_parent(&self.path)?;
        let path = parent.join(OPERATION_LOCK_FILE);
        let file = open_lock_file(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(LocalConnectionOperationGuard { file }),
            Err(fs::TryLockError::WouldBlock) => {
                Err(ConnectionRepositoryError::OperationBusy(path))
            },
            Err(fs::TryLockError::Error(source)) => {
                Err(ConnectionRepositoryError::io(&path, source))
            },
        }
    }

    /// Fails closed on a journal from a newer operation implementation.
    pub fn recover_pending_operation(&self) -> Result<(), ConnectionRepositoryError> {
        let Some(parent) = self.path.parent() else {
            return Err(ConnectionRepositoryError::InvalidPath(self.path.clone()));
        };
        let path = parent.join(PENDING_OPERATION_FILE);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(ConnectionRepositoryError::PendingOperation(path)),
            Err(source) => Err(ConnectionRepositoryError::io(&path, source)),
        }
    }

    /// Publishes the exact prepared bytes if the expected revision still owns the path.
    pub fn commit(
        &self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionRepositoryError> {
        let parent = prepare_parent(&self.path)?;
        let lock_path = parent.join(REPOSITORY_LOCK_FILE);
        let lock = open_lock_file(&lock_path)?;
        lock.lock()
            .map_err(|source| ConnectionRepositoryError::io(&lock_path, source))?;

        let current = read_snapshot(&self.path)?;
        if current.revision == mutation.planned_revision
            && current.encoded == mutation.planned_bytes
        {
            return Ok(ConnectionCommit::AlreadyCommitted);
        }
        if current.revision != mutation.expected_revision {
            return Err(ConnectionRepositoryError::Conflict {
                expected: mutation.expected_revision.clone(),
                observed: current.revision,
            });
        }

        let temporary = parent.join(format!(
            ".connections.{}.pending",
            mutation
                .planned_revision
                .to_string()
                .strip_prefix("rev-")
                .unwrap_or("unknown")
        ));
        reject_symlink(&temporary)?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)
            .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
        let publication = (|| {
            file.write_all(&mutation.planned_bytes)
                .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
            file.sync_all()
                .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
            if mutation.expected_revision.is_absent() {
                fs::hard_link(&temporary, &self.path)
                    .map_err(|source| ConnectionRepositoryError::io(&self.path, source))?;
                fs::remove_file(&temporary)
                    .map_err(|source| ConnectionRepositoryError::io(&temporary, source))?;
            } else {
                fs::rename(&temporary, &self.path)
                    .map_err(|source| ConnectionRepositoryError::io(&self.path, source))?;
            }
            fs::File::open(&parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|source| ConnectionRepositoryError::io(&parent, source))?;
            Ok(())
        })();
        if publication.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        publication?;
        Ok(ConnectionCommit::Committed)
    }
}

impl ConnectionRepository for LocalConnectionRepository {
    type OperationGuard = LocalConnectionOperationGuard;

    fn acquire_operation(&self) -> Result<Self::OperationGuard, ConnectionRepositoryError> {
        Self::acquire_operation(self)
    }

    fn recover_pending_operation(&self) -> Result<(), ConnectionRepositoryError> {
        Self::recover_pending_operation(self)
    }

    fn capture(&self) -> Result<ConnectionSnapshot, ConnectionRepositoryError> {
        Self::capture(self)
    }

    fn commit(
        &self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionRepositoryError> {
        Self::commit(self, mutation)
    }
}

#[derive(Debug)]
pub struct LocalConnectionOperationGuard {
    file: fs::File,
}

impl Drop for LocalConnectionOperationGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn read_snapshot(path: &Path) -> Result<ConnectionSnapshot, ConnectionRepositoryError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConnectionSnapshot {
                revision: ConnectionRevision::Absent,
                preference: None,
                encoded: Vec::new(),
            });
        },
        Err(source) => return Err(ConnectionRepositoryError::io(path, source)),
    };
    let before = MetadataSnapshot::capture(path, &file)?;
    before.validate(path)?;
    let mut encoded = Vec::with_capacity(
        usize::try_from(before.len.min(MAX_CONNECTION_BYTES))
            .unwrap_or(MAX_CONNECTION_BYTES as usize),
    );
    Read::by_ref(&mut file)
        .take(MAX_CONNECTION_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|source| ConnectionRepositoryError::io(path, source))?;
    if encoded.len() as u64 > MAX_CONNECTION_BYTES {
        return Err(ConnectionRepositoryError::TooLarge(path.to_owned()));
    }
    let after = MetadataSnapshot::capture(path, &file)?;
    if before != after {
        return Err(ConnectionRepositoryError::Changed(path.to_owned()));
    }
    let decoded = wire::decode(path, &encoded)?;
    Ok(ConnectionSnapshot {
        revision: decoded.revision,
        preference: decoded.preference,
        encoded,
    })
}

fn prepare_parent(path: &Path) -> Result<PathBuf, ConnectionRepositoryError> {
    let parent = path
        .parent()
        .ok_or_else(|| ConnectionRepositoryError::InvalidPath(path.to_owned()))?;
    if let Ok(metadata) = fs::symlink_metadata(parent)
        && metadata.file_type().is_symlink()
    {
        return Err(ConnectionRepositoryError::UnsupportedFileType(
            parent.to_owned(),
        ));
    }
    let existed = parent.exists();
    fs::create_dir_all(parent).map_err(|source| ConnectionRepositoryError::io(parent, source))?;
    if !existed {
        fs::set_permissions(parent, fs::Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|source| ConnectionRepositoryError::io(parent, source))?;
    }
    Ok(parent.to_owned())
}

fn open_lock_file(path: &Path) -> Result<fs::File, ConnectionRepositoryError> {
    reject_symlink(path)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(FILE_MODE)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|source| ConnectionRepositoryError::io(path, source))?;
    let metadata = MetadataSnapshot::capture(path, &file)?;
    metadata.validate(path)?;
    Ok(file)
}

fn reject_symlink(path: &Path) -> Result<(), ConnectionRepositoryError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(ConnectionRepositoryError::UnsupportedFileType(
            path.to_owned(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MetadataSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    user: u32,
    group: u32,
    len: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl MetadataSnapshot {
    fn capture(path: &Path, file: &fs::File) -> Result<Self, ConnectionRepositoryError> {
        let metadata = file
            .metadata()
            .map_err(|source| ConnectionRepositoryError::io(path, source))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            user: metadata.uid(),
            group: metadata.gid(),
            len: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }

    fn validate(&self, path: &Path) -> Result<(), ConnectionRepositoryError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(ConnectionRepositoryError::UnsupportedFileType(
                path.to_owned(),
            ));
        }
        if self.user != rustix::process::geteuid().as_raw() {
            return Err(ConnectionRepositoryError::WrongOwner(path.to_owned()));
        }
        if self.mode & 0o077 != 0 {
            return Err(ConnectionRepositoryError::InsecurePermissions(
                path.to_owned(),
            ));
        }
        if self.len > MAX_CONNECTION_BYTES {
            return Err(ConnectionRepositoryError::TooLarge(path.to_owned()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
