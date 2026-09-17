use std::{
    fs,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use rustix::process;

use super::super::{MAX_CONNECTION_BYTES, error::ConnectionRepositoryError};

pub(super) const FILE_MODE: u32 = 0o600;
pub(super) const DIRECTORY_MODE: u32 = 0o700;
#[cfg(target_vendor = "apple")]
pub(super) const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
pub(super) const REGULAR_FILE_MODE: u32 = libc::S_IFREG;
#[cfg(target_vendor = "apple")]
pub(super) const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
pub(super) const FILE_TYPE_MASK: u32 = libc::S_IFMT;
pub(super) const REPOSITORY_LOCK_FILE: &str = ".connections.lock";
pub(super) const OPERATION_LOCK_FILE: &str = ".connection-operation.lock";
pub(super) const PENDING_OPERATION_FILE: &str = "connection-operation.yaml";

pub(super) fn prepare_parent(path: &Path) -> Result<PathBuf, ConnectionRepositoryError> {
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

pub(super) fn open_lock_file(path: &Path) -> Result<fs::File, ConnectionRepositoryError> {
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

pub(super) fn reject_symlink(path: &Path) -> Result<(), ConnectionRepositoryError> {
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
pub(super) struct MetadataSnapshot {
    device: u64,
    inode: u64,
    mode: u32,
    user: u32,
    group: u32,
    pub(super) len: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl MetadataSnapshot {
    pub(super) fn capture(path: &Path, file: &fs::File) -> Result<Self, ConnectionRepositoryError> {
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

    pub(super) fn validate(&self, path: &Path) -> Result<(), ConnectionRepositoryError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(ConnectionRepositoryError::UnsupportedFileType(
                path.to_owned(),
            ));
        }
        if self.user != process::geteuid().as_raw() {
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
