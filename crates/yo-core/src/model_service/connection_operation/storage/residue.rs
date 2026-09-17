//! pending 저널 잔류물의 이름 규칙, 검증, 정리 경계.

use std::{
    ffi::OsStr,
    fs,
    io::Result as IoResult,
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

use rustix::process;

use super::{
    super::ConnectionOperationError,
    security::{FileIdentity, MetadataSnapshot},
};

const PENDING_PREFIX: &[u8] = b".connection-operation.";
const PENDING_SUFFIX: &[u8] = b".pending";
const PENDING_HEX_BYTES: usize = 32;

pub(in super::super) fn pending_residue_path(parent: &Path, random: [u8; 16]) -> PathBuf {
    const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";

    let mut name =
        Vec::with_capacity(PENDING_PREFIX.len() + PENDING_HEX_BYTES + PENDING_SUFFIX.len());
    name.extend_from_slice(PENDING_PREFIX);
    for byte in random {
        name.push(LOWER_HEX[usize::from(byte >> 4)]);
        name.push(LOWER_HEX[usize::from(byte & 0x0f)]);
    }
    name.extend_from_slice(PENDING_SUFFIX);
    parent.join(OsStr::from_bytes(&name))
}

pub(in super::super) fn cleanup_pending_residues(
    parent: &Path,
) -> Result<(), ConnectionOperationError> {
    cleanup_pending_residues_with(
        parent,
        validate_pending_residues,
        next_pending_residue,
        |path| fs::remove_file(path),
        |directory| directory.sync_all(),
    )
}

fn cleanup_pending_residues_with(
    parent: &Path,
    validate_all: impl FnOnce(&Path) -> Result<(), ConnectionOperationError>,
    mut next_candidate: impl FnMut(&Path) -> Result<Option<PathBuf>, ConnectionOperationError>,
    mut remove_file: impl FnMut(&Path) -> IoResult<()>,
    mut sync_directory: impl FnMut(&fs::File) -> IoResult<()>,
) -> Result<(), ConnectionOperationError> {
    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(parent)
        .map_err(|source| ConnectionOperationError::io(parent, source))?;
    let directory_identity = FileIdentity::capture(parent, &directory)?;
    directory_identity.revalidate_directory(parent)?;

    validate_all(parent)?;
    directory_identity.revalidate_directory(parent)?;

    let mut removed = false;
    let removal = (|| {
        while let Some(candidate) = next_candidate(parent)? {
            capture_pending_residue(&candidate)?;
            remove_file(&candidate)
                .map_err(|source| ConnectionOperationError::io(&candidate, source))?;
            removed = true;
        }
        Ok(())
    })();
    if removed {
        let synchronization = sync_directory(&directory)
            .map_err(|source| ConnectionOperationError::io(parent, source));
        let revalidation = directory_identity.revalidate_directory(parent);
        synchronization?;
        revalidation?;
    } else if removal.is_ok() {
        directory_identity.revalidate_directory(parent)?;
    }
    removal
}

fn validate_pending_residues(parent: &Path) -> Result<(), ConnectionOperationError> {
    for entry in
        fs::read_dir(parent).map_err(|source| ConnectionOperationError::io(parent, source))?
    {
        let entry = entry.map_err(|source| ConnectionOperationError::io(parent, source))?;
        if pending_residue_name(entry.file_name().as_os_str()) {
            capture_pending_residue(&entry.path())?;
        }
    }
    Ok(())
}

fn next_pending_residue(parent: &Path) -> Result<Option<PathBuf>, ConnectionOperationError> {
    for entry in
        fs::read_dir(parent).map_err(|source| ConnectionOperationError::io(parent, source))?
    {
        let entry = entry.map_err(|source| ConnectionOperationError::io(parent, source))?;
        if pending_residue_name(entry.file_name().as_os_str()) {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

#[cfg(test)]
pub(in super::super) fn cleanup_pending_residues_in_order_for_test(
    parent: &Path,
    validation_order: &[PathBuf],
    removal_order: &[PathBuf],
    remove_file: impl FnMut(&Path) -> IoResult<()>,
    sync_directory: impl FnMut(&fs::File) -> IoResult<()>,
) -> Result<(), ConnectionOperationError> {
    let mut removal_order = removal_order.iter();
    cleanup_pending_residues_with(
        parent,
        |_| {
            for candidate in validation_order {
                capture_pending_residue(candidate)?;
            }
            Ok(())
        },
        |_| Ok(removal_order.next().cloned()),
        remove_file,
        sync_directory,
    )
}

fn pending_residue_name(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    let expected_len = PENDING_PREFIX.len() + PENDING_HEX_BYTES + PENDING_SUFFIX.len();
    bytes.len() == expected_len
        && bytes.starts_with(PENDING_PREFIX)
        && bytes.ends_with(PENDING_SUFFIX)
        && bytes[PENDING_PREFIX.len()..PENDING_PREFIX.len() + PENDING_HEX_BYTES]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

#[cfg(test)]
pub(in super::super) fn pending_residue_name_for_test(name: &OsStr) -> bool {
    pending_residue_name(name)
}

#[cfg(test)]
pub(in super::super) fn pending_residue_path_for_test(parent: &Path, random: [u8; 16]) -> PathBuf {
    pending_residue_path(parent, random)
}

fn capture_pending_residue(path: &Path) -> Result<MetadataSnapshot, ConnectionOperationError> {
    capture_pending_residue_for_user(path, process::geteuid().as_raw())
}

fn capture_pending_residue_for_user(
    path: &Path,
    expected_user: u32,
) -> Result<MetadataSnapshot, ConnectionOperationError> {
    let pathname_before = fs::symlink_metadata(path)
        .map(|metadata| MetadataSnapshot::from_metadata(&metadata))
        .map_err(|source| ConnectionOperationError::io(path, source))?;
    pathname_before.validate_pending_residue_for_user(path, expected_user)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|source| ConnectionOperationError::io(path, source))?;
    let descriptor = MetadataSnapshot::capture(path, &file)?;
    descriptor.validate_pending_residue_for_user(path, expected_user)?;
    let pathname_after = fs::symlink_metadata(path)
        .map(|metadata| MetadataSnapshot::from_metadata(&metadata))
        .map_err(|source| ConnectionOperationError::io(path, source))?;
    if pathname_before != descriptor || descriptor != pathname_after {
        return Err(ConnectionOperationError::Changed(path.to_owned()));
    }
    Ok(descriptor)
}

#[cfg(test)]
pub(in super::super) fn validate_pending_residue_owner_for_test(
    path: &Path,
    expected_user: u32,
) -> Result<(), ConnectionOperationError> {
    capture_pending_residue_for_user(path, expected_user).map(|_| ())
}
