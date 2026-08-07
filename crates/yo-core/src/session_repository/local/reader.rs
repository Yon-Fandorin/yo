use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::{
    super::{RepositoryEntry, RepositoryError, RepositorySequence, SessionRecordVersion},
    file::{
        legacy_writer_is_active, pending_append_is_active, reject_symlink, require_user_only_file,
        scan_complete_entries, session_writer_is_active,
    },
    wire::WireEntry,
};
use crate::SessionId;

const MAX_PENDING_MARKER_GENERATIONS: usize = 4;

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
    let cutoff = guarded_cutoff(root, path, expected_session)?.unwrap_or(physical_len);
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
) -> Result<Option<Vec<RepositoryEntry>>, RepositoryError> {
    reject_symlink(path)?;
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    require_user_only_file(&file)?;
    let physical_len = file.metadata()?.len();
    let cutoff = guarded_cutoff(root, path, expected_session)?.unwrap_or(physical_len);
    if cutoff > physical_len {
        return Err(RepositoryError::Quarantined {
            message: "Session append marker points beyond the physical log".to_owned(),
        });
    }
    let mut reader = BufReader::new(file.take(cutoff));
    Ok(Some(
        scan_complete_entries(&mut reader, expected_session, after, limit)?.entries,
    ))
}

fn guarded_cutoff(
    root: &Path,
    path: &Path,
    session_id: SessionId,
) -> Result<Option<u64>, RepositoryError> {
    let pending = pending_path(path);
    for _ in 0..MAX_PENDING_MARKER_GENERATIONS {
        reject_symlink(&pending)?;
        let mut marker = match OpenOptions::new().read(true).open(&pending) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        require_user_only_file(&marker)?;
        let mut value = String::new();
        marker.read_to_string(&mut value)?;
        let cutoff = value
            .trim()
            .parse::<u64>()
            .map_err(|_| RepositoryError::Quarantined {
                message: "active Session append marker has an invalid durable cutoff".to_owned(),
            })?;
        let marker_is_active = pending_append_is_active(&marker)?;
        if (marker_is_active && session_writer_is_active(root, session_id)?)
            || legacy_writer_is_active(root)?
        {
            return Ok(Some(cutoff));
        }
        match marker_path_matches(&marker, &pending)? {
            None => return Ok(None),
            Some(false) => continue,
            Some(true) => {
                return Err(RepositoryError::Quarantined {
                    message: format!(
                        "Session log is quarantined by an unfinished append at {}",
                        pending.display()
                    ),
                });
            },
        }
    }
    Err(RepositoryError::Unavailable {
        message: "Session append marker changed repeatedly while it was being observed".to_owned(),
    })
}

fn marker_path_matches(marker: &File, pending: &Path) -> Result<Option<bool>, RepositoryError> {
    let opened = marker.metadata()?;
    let current = match fs::symlink_metadata(pending) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(RepositoryError::Unavailable {
                message: format!("symbolic links are not allowed at {}", pending.display()),
            });
        },
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(
        opened.dev() == current.dev() && opened.ino() == current.ino(),
    ))
}

fn pending_path(path: &Path) -> PathBuf {
    path.with_extension("jsonl.pending")
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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    // Reader가 열어 둔 marker inode와 같은 pathname에 새 marker가 교체되면 이를 유기된
    // 기존 marker가 아니라 다른 append generation으로 구분하는지 검증합니다.
    #[test]
    fn distinguishes_a_replacement_pending_marker_generation() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock follows the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "yo-session-marker-generation-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("the test root is created");
        let pending = root.join("session.jsonl.pending");
        fs::write(&pending, b"0\n").expect("the first marker is written");
        fs::set_permissions(&pending, fs::Permissions::from_mode(0o600))
            .expect("the first marker permissions are restricted");
        let opened = File::open(&pending).expect("the reader opens the first marker");

        assert_eq!(marker_path_matches(&opened, &pending).unwrap(), Some(true));
        fs::remove_file(&pending).expect("the first marker is removed");
        assert_eq!(marker_path_matches(&opened, &pending).unwrap(), None);
        fs::write(&pending, b"1\n").expect("the replacement marker is written");
        fs::set_permissions(&pending, fs::Permissions::from_mode(0o600))
            .expect("the replacement marker permissions are restricted");
        assert_eq!(marker_path_matches(&opened, &pending).unwrap(), Some(false));

        drop(opened);
        fs::remove_dir_all(root).expect("the test root is removed");
    }
}
