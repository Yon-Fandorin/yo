//! 경로, 파일 메타데이터, 권한, inode identity, 디렉터리 동기화 경계.

use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use rustix::process;

use super::super::{ConnectionOperationError, error::MAX_OPERATION_JOURNAL_BYTES};

pub(super) const FILE_MODE: u32 = 0o600;
const DIRECTORY_MODE: u32 = 0o700;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;
#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;

pub(super) fn prepare_parent(path: &Path) -> Result<PathBuf, ConnectionOperationError> {
    let parent = path
        .parent()
        .ok_or_else(|| ConnectionOperationError::InvalidPath(path.to_owned()))?;
    if let Ok(metadata) = fs::symlink_metadata(parent)
        && metadata.file_type().is_symlink()
    {
        return Err(ConnectionOperationError::UnsupportedFileType(
            parent.to_owned(),
        ));
    }
    let existed = parent.exists();
    fs::create_dir_all(parent).map_err(|source| ConnectionOperationError::io(parent, source))?;
    if !existed {
        fs::set_permissions(parent, fs::Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|source| ConnectionOperationError::io(parent, source))?;
    }
    Ok(parent.to_owned())
}

pub(super) fn sync_directory(path: &Path) -> Result<(), ConnectionOperationError> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| ConnectionOperationError::io(path, source))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MetadataSnapshot {
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
    pub(super) const fn len(&self) -> u64 {
        self.len
    }

    pub(super) fn capture(path: &Path, file: &fs::File) -> Result<Self, ConnectionOperationError> {
        let metadata = file
            .metadata()
            .map_err(|source| ConnectionOperationError::io(path, source))?;
        Ok(Self::from_metadata(&metadata))
    }

    pub(super) fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
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
        }
    }

    pub(super) fn validate(&self, path: &Path) -> Result<(), ConnectionOperationError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(ConnectionOperationError::UnsupportedFileType(
                path.to_owned(),
            ));
        }
        if self.user != process::geteuid().as_raw() {
            return Err(ConnectionOperationError::WrongOwner(path.to_owned()));
        }
        if self.mode & 0o077 != 0 {
            return Err(ConnectionOperationError::InsecurePermissions(
                path.to_owned(),
            ));
        }
        if self.len > MAX_OPERATION_JOURNAL_BYTES {
            return Err(ConnectionOperationError::TooLarge(path.to_owned()));
        }
        Ok(())
    }

    pub(super) fn validate_pending_residue_for_user(
        &self,
        path: &Path,
        expected_user: u32,
    ) -> Result<(), ConnectionOperationError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(ConnectionOperationError::UnsupportedFileType(
                path.to_owned(),
            ));
        }
        if self.user != expected_user {
            return Err(ConnectionOperationError::WrongOwner(path.to_owned()));
        }
        if self.mode & 0o7777 != FILE_MODE {
            return Err(ConnectionOperationError::InsecurePermissions(
                path.to_owned(),
            ));
        }
        if self.len > MAX_OPERATION_JOURNAL_BYTES {
            return Err(ConnectionOperationError::TooLarge(path.to_owned()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    pub(super) fn capture(path: &Path, file: &fs::File) -> Result<Self, ConnectionOperationError> {
        let metadata = file
            .metadata()
            .map_err(|source| ConnectionOperationError::io(path, source))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    pub(super) fn revalidate_directory(self, path: &Path) -> Result<(), ConnectionOperationError> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|source| ConnectionOperationError::io(path, source))?;
        if !metadata.file_type().is_dir()
            || self.device != metadata.dev()
            || self.inode != metadata.ino()
        {
            return Err(ConnectionOperationError::Changed(path.to_owned()));
        }
        Ok(())
    }
}
