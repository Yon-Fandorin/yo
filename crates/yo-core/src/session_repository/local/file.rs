use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::{
    super::{DurableRecordKind, RepositoryEntry, RepositoryError, RepositorySequence},
    WireEntry,
};
use crate::SessionId;

const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

#[derive(Debug)]
pub(super) struct WriterLock {
    _file: File,
}

impl WriterLock {
    pub(super) fn acquire(root: &Path) -> Result<Self, RepositoryError> {
        let path = root.join(".writer.lock");
        reject_symlink(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(FILE_MODE)
            .open(&path)?;
        require_user_only_file(&file)?;
        file.try_lock()
            .map_err(|error| RepositoryError::Unavailable {
                message: format!("another writer owns the Session repository: {error}"),
            })?;
        Ok(Self { _file: file })
    }
}

pub(super) struct ScanResult {
    pub(super) durable_cutoff: Option<RepositorySequence>,
    pub(super) journal_cutoff: Option<crate::JournalSequence>,
    pub(super) entries: Vec<RepositoryEntry>,
}

pub(super) fn prepare_root(root: &Path) -> Result<PathBuf, RepositoryError> {
    if let Ok(metadata) = fs::symlink_metadata(root)
        && metadata.file_type().is_symlink()
    {
        return Err(RepositoryError::Unavailable {
            message: "Session repository root must not be a symbolic link".to_owned(),
        });
    }
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

pub(super) fn append_line(root: &Path, path: &Path, encoded: &[u8]) -> Result<(), RepositoryError> {
    reject_symlink(path)?;
    let pending = pending_path(path);
    begin_pending_append(root, &pending)?;
    let existed = path.try_exists()?;
    let mut file = match OpenOptions::new()
        .create(true)
        .append(true)
        .mode(FILE_MODE)
        .open(path)
    {
        Ok(file) => file,
        Err(error) => {
            clear_pending_append(root, &pending)?;
            return Err(error.into());
        },
    };
    require_user_only_file(&file)?;
    let durable_bytes = file.metadata()?.len();
    let append = file
        .write_all(encoded)
        .and_then(|()| file.sync_data())
        .and_then(|()| {
            if existed && durable_bytes != 0 {
                Ok(())
            } else {
                File::open(root)?.sync_all()
            }
        });
    if let Err(error) = append {
        let rollback = file.set_len(durable_bytes).and_then(|()| file.sync_data());
        let message = match rollback {
            Ok(()) => match clear_pending_append(root, &pending) {
                Ok(()) => format!("failed to append a Session record: {error}"),
                Err(clear) => format!(
                    "failed to append a Session record ({error}) and clear its pending marker ({clear})"
                ),
            },
            Err(rollback) => format!(
                "failed to append a Session record ({error}) and roll back its tail ({rollback}); \
                 the Session log remains quarantined"
            ),
        };
        return Err(RepositoryError::Unavailable { message });
    }
    clear_pending_append(root, &pending)?;
    Ok(())
}

pub(super) fn scan_entries(
    path: &Path,
    expected_session: SessionId,
    repair_tail: bool,
    after: u64,
    limit: usize,
) -> Result<ScanResult, RepositoryError> {
    reject_symlink(path)?;
    reject_pending_append(path)?;
    let file = match OpenOptions::new().read(true).write(repair_tail).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ScanResult {
                durable_cutoff: None,
                journal_cutoff: None,
                entries: Vec::new(),
            });
        },
        Err(error) => return Err(error.into()),
    };
    require_user_only_file(&file)?;

    let mut reader = BufReader::new(file);
    let mut entries = Vec::with_capacity(limit.min(256));
    let mut line = Vec::new();
    let mut durable_bytes = 0_u64;
    let mut line_number = 0_usize;
    let mut durable_cutoff = None;
    let mut journal_cutoff = None;

    loop {
        line.clear();
        let bytes = reader.read_until(b'\n', &mut line)?;
        if bytes == 0 {
            break;
        }
        line_number = line_number.saturating_add(1);
        if !line.ends_with(b"\n") {
            if repair_tail {
                reader.get_mut().set_len(durable_bytes)?;
                reader.get_mut().seek(SeekFrom::Start(durable_bytes))?;
                reader.get_mut().sync_data()?;
            }
            break;
        }

        let wire: WireEntry =
            serde_json::from_slice(&line).map_err(|error| RepositoryError::CorruptLog {
                line: line_number,
                reason: error.to_string(),
            })?;
        let expected_sequence = durable_cutoff.map_or(1, |sequence: RepositorySequence| {
            sequence.get().saturating_add(1)
        });
        let entry = wire.into_record(expected_session, expected_sequence, line_number)?;
        durable_cutoff = Some(entry.sequence());
        if let Some(sequence) = entry.record().journal_cutoff() {
            let invalid = journal_cutoff.is_some_and(|previous| {
                sequence < previous
                    || (sequence == previous
                        && entry.record().kind() != DurableRecordKind::Snapshot)
            });
            if invalid {
                return Err(RepositoryError::CorruptLog {
                    line: line_number,
                    reason: "Journal sequence does not advance independently".to_owned(),
                });
            }
            journal_cutoff = Some(sequence);
        }
        if entry.sequence().get() > after && entries.len() < limit {
            entries.push(entry);
        }
        durable_bytes = durable_bytes.saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    Ok(ScanResult {
        durable_cutoff,
        journal_cutoff,
        entries,
    })
}

fn pending_path(path: &Path) -> PathBuf {
    path.with_extension("jsonl.pending")
}

fn begin_pending_append(root: &Path, pending: &Path) -> Result<(), RepositoryError> {
    reject_symlink(pending)?;
    let mut marker = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(FILE_MODE)
        .open(pending)
        .map_err(|error| RepositoryError::Unavailable {
            message: if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "Session log is quarantined by an unfinished append at {}",
                    pending.display()
                )
            } else {
                format!(
                    "failed to create the Session append marker at {}: {error}",
                    pending.display()
                )
            },
        })?;
    marker.write_all(b"pending\n")?;
    marker.sync_data()?;
    File::open(root)?.sync_all()?;
    Ok(())
}

fn clear_pending_append(root: &Path, pending: &Path) -> Result<(), RepositoryError> {
    fs::remove_file(pending)?;
    // The Session data was already synced. If syncing the marker removal fails,
    // a crash may conservatively restore the marker and quarantine valid data,
    // but it cannot expose a failed append as committed.
    let _ = File::open(root).and_then(|directory| directory.sync_all());
    Ok(())
}

fn reject_pending_append(path: &Path) -> Result<(), RepositoryError> {
    let pending = pending_path(path);
    reject_symlink(&pending)?;
    if pending.try_exists()? {
        Err(RepositoryError::Unavailable {
            message: format!(
                "Session log is quarantined by an unfinished append at {}",
                pending.display()
            ),
        })
    } else {
        Ok(())
    }
}

fn reject_symlink(path: &Path) -> Result<(), RepositoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RepositoryError::Unavailable {
            message: format!("symbolic links are not allowed at {}", path.display()),
        }),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn require_user_only_file(file: &File) -> Result<(), RepositoryError> {
    let mode = file.metadata()?.permissions().mode() & 0o777;
    if mode & 0o077 == 0 {
        Ok(())
    } else {
        Err(RepositoryError::Unavailable {
            message: format!("Session repository file permissions {mode:o} are not user-only"),
        })
    }
}
