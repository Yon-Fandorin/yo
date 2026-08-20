use std::{
    ffi::OsStr,
    fs,
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use super::{
    ConnectionOperationError, ConnectionOperationJournalEntry, ConnectionOperationPhase,
    error::MAX_OPERATION_JOURNAL_BYTES, wire,
};

const FILE_MODE: u32 = 0o600;
const DIRECTORY_MODE: u32 = 0o700;
const PENDING_PREFIX: &[u8] = b".connection-operation.";
const PENDING_SUFFIX: &[u8] = b".pending";
const PENDING_HEX_BYTES: usize = 32;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;
#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;

pub(super) fn capture(
    path: &Path,
) -> Result<Option<ConnectionOperationJournalEntry>, ConnectionOperationError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(ConnectionOperationError::io(path, source)),
    };
    let before = MetadataSnapshot::capture(path, &file)?;
    before.validate(path)?;
    let mut encoded = Vec::with_capacity(
        usize::try_from(before.len.min(MAX_OPERATION_JOURNAL_BYTES))
            .unwrap_or(MAX_OPERATION_JOURNAL_BYTES as usize),
    );
    Read::by_ref(&mut file)
        .take(MAX_OPERATION_JOURNAL_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|source| ConnectionOperationError::io(path, source))?;
    if encoded.len() as u64 > MAX_OPERATION_JOURNAL_BYTES {
        return Err(ConnectionOperationError::TooLarge(path.to_owned()));
    }
    let after = MetadataSnapshot::capture(path, &file)?;
    if before != after {
        return Err(ConnectionOperationError::Changed(path.to_owned()));
    }
    wire::decode(path, &encoded).map(Some)
}

pub(super) fn publish_intent(
    path: &Path,
    entry: &ConnectionOperationJournalEntry,
) -> Result<(), ConnectionOperationError> {
    if entry.phase() != ConnectionOperationPhase::Intent {
        return Err(ConnectionOperationError::InvalidEntry);
    }
    let parent = prepare_parent(path)?;
    cleanup_pending_residues(&parent)?;
    let encoded = encode_bounded(entry)?;
    let (temporary, mut file) = create_temporary(&parent)?;
    let publication = (|| {
        file.write_all(&encoded)
            .map_err(|source| ConnectionOperationError::io(&temporary, source))?;
        file.sync_all()
            .map_err(|source| ConnectionOperationError::io(&temporary, source))?;
        match fs::hard_link(&temporary, path) {
            Ok(()) => {},
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(ConnectionOperationError::Conflict(path.to_owned()));
            },
            Err(source) => return Err(ConnectionOperationError::io(path, source)),
        }
        fs::remove_file(&temporary)
            .map_err(|source| ConnectionOperationError::io(&temporary, source))?;
        sync_directory(&parent)?;
        Ok(())
    })();
    if publication.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    publication
}

pub(super) fn advance(
    path: &Path,
    current: &ConnectionOperationJournalEntry,
    next: ConnectionOperationPhase,
) -> Result<ConnectionOperationJournalEntry, ConnectionOperationError> {
    let advanced = current.with_phase(next)?;
    let observed =
        capture(path)?.ok_or_else(|| ConnectionOperationError::Conflict(path.to_owned()))?;
    if &observed != current {
        return Err(ConnectionOperationError::Conflict(path.to_owned()));
    }
    let parent = path
        .parent()
        .ok_or_else(|| ConnectionOperationError::InvalidPath(path.to_owned()))?;
    cleanup_pending_residues(parent)?;
    let encoded = encode_bounded(&advanced)?;
    let (temporary, mut file) = create_temporary(parent)?;
    let publication = (|| {
        file.write_all(&encoded)
            .map_err(|source| ConnectionOperationError::io(&temporary, source))?;
        file.sync_all()
            .map_err(|source| ConnectionOperationError::io(&temporary, source))?;
        fs::rename(&temporary, path)
            .map_err(|source| ConnectionOperationError::io(path, source))?;
        sync_directory(parent)?;
        Ok(())
    })();
    if publication.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    publication?;
    Ok(advanced)
}

pub(super) fn clear_complete(
    path: &Path,
    current: &ConnectionOperationJournalEntry,
) -> Result<(), ConnectionOperationError> {
    if current.phase() != ConnectionOperationPhase::Complete {
        return Err(ConnectionOperationError::InvalidEntry);
    }
    clear_exact(path, current)
}

pub(super) fn abandon_intent(
    path: &Path,
    current: &ConnectionOperationJournalEntry,
) -> Result<(), ConnectionOperationError> {
    if current.phase() != ConnectionOperationPhase::Intent {
        return Err(ConnectionOperationError::InvalidEntry);
    }
    clear_exact(path, current)
}

fn clear_exact(
    path: &Path,
    current: &ConnectionOperationJournalEntry,
) -> Result<(), ConnectionOperationError> {
    let observed =
        capture(path)?.ok_or_else(|| ConnectionOperationError::Conflict(path.to_owned()))?;
    if &observed != current {
        return Err(ConnectionOperationError::Conflict(path.to_owned()));
    }
    fs::remove_file(path).map_err(|source| ConnectionOperationError::io(path, source))?;
    let parent = path
        .parent()
        .ok_or_else(|| ConnectionOperationError::InvalidPath(path.to_owned()))?;
    sync_directory(parent)
}

fn encode_bounded(
    entry: &ConnectionOperationJournalEntry,
) -> Result<Vec<u8>, ConnectionOperationError> {
    let encoded = wire::encode(entry)?;
    if encoded.len() as u64 > MAX_OPERATION_JOURNAL_BYTES {
        return Err(ConnectionOperationError::PreparedTooLarge);
    }
    Ok(encoded)
}

fn prepare_parent(path: &Path) -> Result<PathBuf, ConnectionOperationError> {
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

fn create_temporary(parent: &Path) -> Result<(PathBuf, fs::File), ConnectionOperationError> {
    for _ in 0..16 {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| ConnectionOperationError::Randomness(error.to_string()))?;
        let temporary = pending_residue_path(parent, random);
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(FILE_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(source) => return Err(ConnectionOperationError::io(&temporary, source)),
        }
    }
    Err(ConnectionOperationError::InvalidEntry)
}

fn pending_residue_path(parent: &Path, random: [u8; 16]) -> PathBuf {
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

pub(super) fn cleanup_pending_residues(parent: &Path) -> Result<(), ConnectionOperationError> {
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
    mut remove_file: impl FnMut(&Path) -> std::io::Result<()>,
    mut sync_directory: impl FnMut(&fs::File) -> std::io::Result<()>,
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
pub(super) fn cleanup_pending_residues_in_order_for_test(
    parent: &Path,
    validation_order: &[PathBuf],
    removal_order: &[PathBuf],
    remove_file: impl FnMut(&Path) -> std::io::Result<()>,
    sync_directory: impl FnMut(&fs::File) -> std::io::Result<()>,
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
pub(super) fn pending_residue_name_for_test(name: &OsStr) -> bool {
    pending_residue_name(name)
}

#[cfg(test)]
pub(super) fn pending_residue_path_for_test(parent: &Path, random: [u8; 16]) -> PathBuf {
    pending_residue_path(parent, random)
}

fn capture_pending_residue(path: &Path) -> Result<MetadataSnapshot, ConnectionOperationError> {
    capture_pending_residue_for_user(path, rustix::process::geteuid().as_raw())
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
pub(super) fn validate_pending_residue_owner_for_test(
    path: &Path,
    expected_user: u32,
) -> Result<(), ConnectionOperationError> {
    capture_pending_residue_for_user(path, expected_user).map(|_| ())
}

fn sync_directory(path: &Path) -> Result<(), ConnectionOperationError> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| ConnectionOperationError::io(path, source))
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
    fn capture(path: &Path, file: &fs::File) -> Result<Self, ConnectionOperationError> {
        let metadata = file
            .metadata()
            .map_err(|source| ConnectionOperationError::io(path, source))?;
        Ok(Self::from_metadata(&metadata))
    }

    fn from_metadata(metadata: &fs::Metadata) -> Self {
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

    fn validate(&self, path: &Path) -> Result<(), ConnectionOperationError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(ConnectionOperationError::UnsupportedFileType(
                path.to_owned(),
            ));
        }
        if self.user != rustix::process::geteuid().as_raw() {
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

    fn validate_pending_residue_for_user(
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
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn capture(path: &Path, file: &fs::File) -> Result<Self, ConnectionOperationError> {
        let metadata = file
            .metadata()
            .map_err(|source| ConnectionOperationError::io(path, source))?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn revalidate_directory(self, path: &Path) -> Result<(), ConnectionOperationError> {
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
