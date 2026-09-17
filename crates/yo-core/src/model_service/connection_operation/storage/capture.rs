//! 저널 바이트를 캡처하고 원자적으로 게시하는 저장 경계.

use std::{
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use super::{
    super::{
        ConnectionOperationError, ConnectionOperationJournalEntry, ConnectionOperationPhase,
        error::MAX_OPERATION_JOURNAL_BYTES, wire,
    },
    residue::{cleanup_pending_residues, pending_residue_path},
    security::{FILE_MODE, MetadataSnapshot, prepare_parent, sync_directory},
};

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
            Err(source) if source.kind() == ErrorKind::AlreadyExists => {},
            Err(source) => return Err(ConnectionOperationError::io(&temporary, source)),
        }
    }
    Err(ConnectionOperationError::InvalidEntry)
}

pub(in super::super) fn capture(
    path: &Path,
) -> Result<Option<ConnectionOperationJournalEntry>, ConnectionOperationError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(ConnectionOperationError::io(path, source)),
    };
    let before = MetadataSnapshot::capture(path, &file)?;
    before.validate(path)?;
    let mut encoded = Vec::with_capacity(
        usize::try_from(before.len().min(MAX_OPERATION_JOURNAL_BYTES))
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

pub(in super::super) fn publish_intent(
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
            Err(source) if source.kind() == ErrorKind::AlreadyExists => {
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

pub(in super::super) fn advance(
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

pub(in super::super) fn clear_complete(
    path: &Path,
    current: &ConnectionOperationJournalEntry,
) -> Result<(), ConnectionOperationError> {
    if current.phase() != ConnectionOperationPhase::Complete {
        return Err(ConnectionOperationError::InvalidEntry);
    }
    clear_exact(path, current)
}

pub(in super::super) fn abandon_intent(
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
