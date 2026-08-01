use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::{
    super::{RepositoryEntry, RepositoryError, RepositorySequence, SessionRecordVersion},
    file::{reject_symlink, require_user_only_file, scan_complete_entries},
    wire::WireEntry,
};
use crate::SessionId;

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

pub(super) fn read_tail_discovery(
    root: &Path,
    path: &Path,
    expected_session: SessionId,
) -> Result<
    Option<(
        RepositorySequence,
        SessionRecordVersion,
        super::super::SessionDiscovery,
    )>,
    RepositoryError,
> {
    reject_symlink(path)?;
    let mut file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    require_user_only_file(&file)?;
    let physical_len = file.metadata()?.len();
    let cutoff = guarded_cutoff(root, path)?.unwrap_or(physical_len);
    if cutoff > physical_len {
        return Err(RepositoryError::Quarantined {
            message: "Session append marker points beyond the physical log".to_owned(),
        });
    }
    let Some(line) = last_complete_line(&mut file, cutoff)? else {
        return Ok(None);
    };
    let wire = WireEntry::decode_tail(&line)?;
    let (decoded, version) = wire.into_tail(expected_session)?;
    Ok(Some((decoded.entry.sequence(), version, decoded.discovery)))
}

pub(super) fn read_snapshot_entries(
    root: &Path,
    path: &Path,
    expected_session: SessionId,
    after: u64,
    limit: usize,
) -> Result<Vec<RepositoryEntry>, RepositoryError> {
    reject_symlink(path)?;
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    require_user_only_file(&file)?;
    let physical_len = file.metadata()?.len();
    let cutoff = guarded_cutoff(root, path)?.unwrap_or(physical_len);
    if cutoff > physical_len {
        return Err(RepositoryError::Quarantined {
            message: "Session append marker points beyond the physical log".to_owned(),
        });
    }
    let mut reader = BufReader::new(file.take(cutoff));
    Ok(scan_complete_entries(&mut reader, expected_session, after, limit)?.entries)
}

fn guarded_cutoff(root: &Path, path: &Path) -> Result<Option<u64>, RepositoryError> {
    let pending = pending_path(path);
    reject_symlink(&pending)?;
    let value = match fs::read_to_string(&pending) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let cutoff = value
        .trim()
        .parse::<u64>()
        .map_err(|_| RepositoryError::Quarantined {
            message: "active Session append marker has an invalid durable cutoff".to_owned(),
        })?;
    if writer_is_active(root)? {
        return Ok(Some(cutoff));
    }
    if pending.try_exists()? {
        Err(RepositoryError::Quarantined {
            message: format!(
                "Session log is quarantined by an unfinished append at {}",
                pending.display()
            ),
        })
    } else {
        // The writer committed and removed the marker while it was being observed.
        Ok(None)
    }
}

fn pending_path(path: &Path) -> PathBuf {
    path.with_extension("jsonl.pending")
}

fn writer_is_active(root: &Path) -> Result<bool, RepositoryError> {
    let path = root.join(".writer.lock");
    reject_symlink(&path)?;
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    require_user_only_file(&file)?;
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            Ok(false)
        },
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

fn last_complete_line(file: &mut File, cutoff: u64) -> Result<Option<Vec<u8>>, RepositoryError> {
    if cutoff == 0 {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(cutoff - 1))?;
    let mut final_byte = [0_u8; 1];
    file.read_exact(&mut final_byte)?;
    let line_end = if final_byte[0] == b'\n' {
        cutoff
    } else {
        match previous_newline(file, cutoff)? {
            Some(position) => position + 1,
            None => return Ok(None),
        }
    };
    let line_start = previous_newline(file, line_end - 1)?.map_or(0, |position| position + 1);
    let length =
        usize::try_from(line_end - line_start).map_err(|_| RepositoryError::Unavailable {
            message: "Session tail envelope is too large to address".to_owned(),
        })?;
    let mut line = vec![0_u8; length];
    file.seek(SeekFrom::Start(line_start))?;
    file.read_exact(&mut line)?;
    Ok(Some(line))
}

fn previous_newline(file: &mut File, before: u64) -> Result<Option<u64>, RepositoryError> {
    const BLOCK: usize = 8 * 1024;
    let mut cursor = before;
    let mut buffer = vec![0_u8; BLOCK];
    while cursor > 0 {
        let length = usize::try_from(cursor.min(BLOCK as u64)).unwrap_or(BLOCK);
        let start = cursor - u64::try_from(length).unwrap_or(0);
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..length])?;
        if let Some(index) = buffer[..length].iter().rposition(|byte| *byte == b'\n') {
            return Ok(Some(start + u64::try_from(index).unwrap_or(0)));
        }
        cursor = start;
    }
    Ok(None)
}
