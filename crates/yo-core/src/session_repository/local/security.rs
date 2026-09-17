//! 로컬 저장소 루트, 컷오프, 경로, 메타데이터 검증.

use std::{
    fs::{self, File, OpenOptions},
    io::{Error, ErrorKind},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use rustix::{
    fs::{Mode, OFlags, open, openat},
    io::Errno,
};

use super::super::{DurableCutoff, RepositoryError, RepositorySequence};
use crate::JournalSequence;

pub(super) const DIRECTORY_MODE: u32 = 0o700;
pub(super) const FILE_MODE: u32 = 0o600;

pub(super) fn validate_repository_root(root: &Path) -> Result<(), RepositoryError> {
    if root.as_os_str().is_empty() || !root.is_absolute() {
        return Err(RepositoryError::Unavailable {
            message: "Session repository root must be a non-empty absolute path".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn open_lock_file(path: &Path) -> Result<File, RepositoryError> {
    reject_symlink(path)?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(FILE_MODE)
        .open(path)?;
    require_user_only_file(&file)?;
    Ok(file)
}

pub(super) fn pin_reader_root(root: &Path) -> Result<File, RepositoryError> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open("/", flags, Mode::empty()).map_err(Error::from)?;
    for component in root.components() {
        match component {
            Component::RootDir | Component::CurDir => {},
            Component::Normal(name) => {
                directory = openat(&directory, name, flags, Mode::empty()).map_err(Error::from)?;
            },
            Component::ParentDir => {
                directory = openat(&directory, "..", flags, Mode::empty()).map_err(Error::from)?;
            },
            _ => {
                return Err(RepositoryError::Unavailable {
                    message: "Session reader root must be a canonical absolute path".into(),
                });
            },
        }
    }
    let file = File::from(directory);
    require_user_only_file(&file)?;
    Ok(file)
}

pub(super) fn open_readonly_regular_at(
    root: &File,
    name: &Path,
) -> Result<Option<File>, RepositoryError> {
    let opened = match openat(
        root,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(Errno::NOENT) => return Ok(None),
        Err(error) => return Err(Error::from(error).into()),
    };
    let file = File::from(opened);
    if !file.metadata()?.is_file() {
        return Err(RepositoryError::Unavailable {
            message: "Session repository entry is not a regular file".into(),
        });
    }
    require_user_only_file(&file)?;
    Ok(Some(file))
}

/// 링크를 따르거나 FIFO를 기다리지 않고 사용자 전용 권한의 기존 일반 파일만 엽니다.
pub(super) fn open_readonly_regular(path: &Path) -> Result<Option<File>, RepositoryError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !file.metadata()?.is_file() {
        return Err(RepositoryError::Unavailable {
            message: format!(
                "Session repository entry is not a regular file: {}",
                path.display()
            ),
        });
    }
    require_user_only_file(&file)?;
    Ok(Some(file))
}

pub(super) fn prepare_root(root: &Path) -> Result<PathBuf, RepositoryError> {
    reject_symlink(root)?;
    fs::create_dir_all(root)?;
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() {
        return Err(RepositoryError::Unavailable {
            message: "Session repository root is not a directory".to_owned(),
        });
    }
    fs::set_permissions(root, fs::Permissions::from_mode(DIRECTORY_MODE))?;
    File::open(root)?.sync_all()?;
    Ok(fs::canonicalize(root)?)
}

pub(super) fn open_existing_root(root: &Path) -> Result<PathBuf, RepositoryError> {
    reject_symlink(root)?;
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() {
        return Err(RepositoryError::Unavailable {
            message: "Session repository root is not a directory".to_owned(),
        });
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(RepositoryError::Unavailable {
            message: "Session repository directory permissions are not user-only".to_owned(),
        });
    }
    Ok(fs::canonicalize(root)?)
}

pub(super) fn reject_symlink(path: &Path) -> Result<(), RepositoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RepositoryError::Unavailable {
            message: format!("symbolic links are not allowed at {}", path.display()),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn require_user_only_file(file: &File) -> Result<(), RepositoryError> {
    let mode = file.metadata()?.permissions().mode() & 0o777;
    if mode & 0o077 == 0 {
        Ok(())
    } else {
        Err(RepositoryError::Unavailable {
            message: format!("Session repository file permissions {mode:o} are not user-only"),
        })
    }
}

pub(super) fn pending_path(path: &Path) -> PathBuf {
    path.with_extension("jsonl.pending")
}

pub(super) fn reject_pending_append(path: &Path) -> Result<(), RepositoryError> {
    let pending = pending_path(path);
    reject_symlink(&pending)?;
    if pending.try_exists()? {
        Err(RepositoryError::Quarantined {
            message: format!(
                "Session log is quarantined by an unfinished append at {}",
                pending.display()
            ),
        })
    } else {
        Ok(())
    }
}

pub(super) fn parse_pending_cutoff(value: &str) -> Result<u64, RepositoryError> {
    if value.len() > 64 {
        return Err(RepositoryError::Quarantined {
            message: "Session append marker exceeds its bounded cutoff encoding".into(),
        });
    }
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| RepositoryError::Quarantined {
            message: "active Session append marker has an invalid durable cutoff".into(),
        })
}

pub(super) fn marker_path_matches(
    marker: &File,
    pending: &Path,
) -> Result<Option<bool>, RepositoryError> {
    let opened = marker.metadata()?;
    let current = match fs::symlink_metadata(pending) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(RepositoryError::Unavailable {
                message: format!("symbolic links are not allowed at {}", pending.display()),
            });
        },
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(file_identity_matches(
        opened.dev(),
        opened.ino(),
        current.dev(),
        current.ino(),
    )))
}

pub(super) fn file_identity_matches<Device: PartialEq, Inode: PartialEq>(
    opened_device: Device,
    opened_inode: Inode,
    current_device: Device,
    current_inode: Inode,
) -> bool {
    opened_device == current_device && opened_inode == current_inode
}

pub(super) fn stable_file_metadata_matches(
    initial: &fs::Metadata,
    final_metadata: &fs::Metadata,
    current_device: u64,
    current_inode: u64,
) -> bool {
    initial.dev() == current_device
        && initial.ino() == current_inode
        && initial.len() == final_metadata.len()
        && initial.mtime() == final_metadata.mtime()
        && initial.mtime_nsec() == final_metadata.mtime_nsec()
        && initial.ctime() == final_metadata.ctime()
        && initial.ctime_nsec() == final_metadata.ctime_nsec()
}

pub(super) fn known_cutoff(
    durable_cutoff: Option<RepositorySequence>,
    journal_cutoff: Option<JournalSequence>,
) -> DurableCutoff {
    match durable_cutoff {
        Some(repository_sequence) => DurableCutoff::Known {
            journal_sequence: journal_cutoff,
            repository_sequence,
        },
        None => DurableCutoff::KnownEmpty,
    }
}

#[cfg(test)]
mod tests {
    use super::file_identity_matches;

    // macOS exposes device and inode numbers with different integer types.
    #[test]
    fn file_identity_compares_device_and_inode_types_independently() {
        assert!(file_identity_matches(7_i32, 11_u64, 7_i32, 11_u64));
        assert!(!file_identity_matches(7_i32, 11_u64, 8_i32, 11_u64));
        assert!(!file_identity_matches(7_i32, 11_u64, 7_i32, 12_u64));
    }
}
