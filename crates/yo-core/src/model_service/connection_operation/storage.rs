use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::{
    ConnectionOperationError, ConnectionOperationJournalEntry, ConnectionOperationPhase,
    error::MAX_OPERATION_JOURNAL_BYTES, wire,
};

const FILE_MODE: u32 = 0o600;
const DIRECTORY_MODE: u32 = 0o700;
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
        let mut suffix = String::with_capacity(32);
        for byte in random {
            use std::fmt::Write as _;
            write!(suffix, "{byte:02x}").expect("formatting into a String cannot fail");
        }
        let temporary = parent.join(format!(".connection-operation.{suffix}.pending"));
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
}
