#[cfg(test)]
use std::cell::RefCell;
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::{self, File},
    io::{BufRead, BufReader, Error, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, PoisonError, Weak},
};

use rustix::{
    fs::{AtFlags, FileType, Mode, OFlags, linkat, openat, statat, unlinkat},
    io::Errno,
};

use super::{
    super::{DurableRecordKind, RepositoryEntry, RepositoryError, RepositorySequence},
    wire::WireEntry,
};
use crate::SessionId;

const LEGACY_WRITER_LOCK: &str = ".writer.lock";
const APPEND_COORDINATOR_LOCK: &str = ".append.lock";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct RepositoryRootIdentity {
    device: u64,
    inode: u64,
}

impl RepositoryRootIdentity {
    pub(super) fn read(root: &Path) -> Result<Self, RepositoryError> {
        let metadata = fs::metadata(root)?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

pub(super) fn process_root_append_coordinator(root: RepositoryRootIdentity) -> Arc<Mutex<()>> {
    static COORDINATORS: OnceLock<Mutex<HashMap<RepositoryRootIdentity, Weak<Mutex<()>>>>> =
        OnceLock::new();

    let coordinators = COORDINATORS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut coordinators = coordinators.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(coordinator) = coordinators.get(&root).and_then(Weak::upgrade) {
        return coordinator;
    }

    coordinators.retain(|_, coordinator| coordinator.strong_count() != 0);
    let coordinator = Arc::new(Mutex::new(()));
    coordinators.insert(root, Arc::downgrade(&coordinator));
    coordinator
}

#[cfg(test)]
thread_local! {
    static AFTER_ROOT_PINNED: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

#[cfg(test)]
pub(in crate::session_repository) fn install_append_root_pinned_hook(
    hook: impl FnOnce() + 'static,
) {
    AFTER_ROOT_PINNED.with(|slot| {
        assert!(
            slot.borrow_mut().replace(Box::new(hook)).is_none(),
            "an append root-pinned hook is already installed"
        );
    });
}

#[derive(Debug)]
pub(super) struct LegacyWriterCompatibilityGuard {
    _file: File,
}

impl LegacyWriterCompatibilityGuard {
    pub(super) fn acquire(root: &Path) -> Result<Self, RepositoryError> {
        let path = root.join(LEGACY_WRITER_LOCK);
        let file = super::security::open_lock_file(&path)?;
        match file.try_lock_shared() {
            Ok(()) => {},
            Err(fs::TryLockError::WouldBlock) => {
                return Err(RepositoryError::Unavailable {
                    message: format!(
                        "a legacy writer owns the Session repository compatibility lock at {}",
                        path.display()
                    ),
                });
            },
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        Ok(Self { _file: file })
    }
}

#[derive(Debug)]
pub(super) struct SessionWriterLease {
    _file: File,
}

impl SessionWriterLease {
    pub(super) fn acquire(root: &Path, session_id: SessionId) -> Result<Self, RepositoryError> {
        let path = session_writer_lock_path(root, session_id);
        let file = super::security::open_lock_file(&path)?;
        match file.try_lock() {
            Ok(()) => {},
            Err(fs::TryLockError::WouldBlock) => {
                return Err(RepositoryError::Unavailable {
                    message: format!("another writer owns Session {session_id}"),
                });
            },
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        Ok(Self { _file: file })
    }
}

#[derive(Debug)]
pub(super) struct RootAppendGuard {
    file: File,
}

#[derive(Debug)]
struct PendingAppendGuard {
    _file: File,
}

impl RootAppendGuard {
    pub(super) fn acquire(root: &Path) -> Result<Self, RepositoryError> {
        let path = root.join(APPEND_COORDINATOR_LOCK);
        let file = super::security::open_lock_file(&path)?;
        file.lock()?;
        Ok(Self { file })
    }
}

impl Drop for RootAppendGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(super) fn coordination_file(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    name == LEGACY_WRITER_LOCK
        || name == APPEND_COORDINATOR_LOCK
        || name.ends_with(".writer.lock")
        || name.ends_with(".pending")
        || name.ends_with(".pending.preparing")
}

pub(super) fn pending_append_is_active(file: &File) -> Result<bool, RepositoryError> {
    exclusive_file_lock_is_active(file)
}

fn session_writer_lock_path(root: &Path, session_id: SessionId) -> PathBuf {
    root.join(format!("{session_id}.writer.lock"))
}

pub(super) fn tree_lock_is_active(root: &File, name: &Path) -> Result<bool, RepositoryError> {
    match super::security::open_readonly_regular_at(root, name)? {
        Some(file) => exclusive_file_lock_is_active(&file),
        None => Ok(false),
    }
}

fn exclusive_file_lock_is_active(file: &File) -> Result<bool, RepositoryError> {
    match file.try_lock_shared() {
        Ok(()) => {
            let _ = file.unlock();
            Ok(false)
        },
        Err(fs::TryLockError::WouldBlock) => Ok(true),
        Err(fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

pub(super) struct ScanResult {
    pub(super) durable_cutoff: Option<RepositorySequence>,
    pub(super) journal_cutoff: Option<crate::JournalSequence>,
    pub(super) entries: Vec<RepositoryEntry>,
    durable_bytes: u64,
}

pub(super) fn append_line(root: &Path, path: &Path, encoded: &[u8]) -> Result<(), RepositoryError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| RepositoryError::Unavailable {
            message: format!(
                "Session log path is outside its repository root: {}",
                path.display()
            ),
        })?;
    if relative
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        return Err(RepositoryError::Unavailable {
            message: format!(
                "Session log path is not a repository entry: {}",
                path.display()
            ),
        });
    }
    let name = relative
        .file_name()
        .ok_or_else(|| RepositoryError::Unavailable {
            message: format!("Session log path has no name: {}", path.display()),
        })?;
    let directory = super::security::pin_reader_root(root)?;
    #[cfg(test)]
    AFTER_ROOT_PINNED.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
    let append_flags = OFlags::WRONLY | OFlags::APPEND | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let (opened, existed) = match openat(
        &directory,
        name,
        append_flags | OFlags::CREATE | OFlags::EXCL,
        super::security::FILE_MODE,
    ) {
        Ok(file) => (file, false),
        Err(Errno::EXIST) => (
            openat(&directory, name, append_flags, Mode::empty()).map_err(Error::from)?,
            true,
        ),
        Err(error) => return Err(Error::from(error).into()),
    };
    let mut file = File::from(opened);
    if !file.metadata()?.is_file() {
        return Err(RepositoryError::Unavailable {
            message: "Session repository entry is not a regular file".into(),
        });
    }
    super::security::require_user_only_file(&file)?;
    let durable_bytes = file.metadata()?.len();
    let pending = super::security::pending_path(relative);
    let pending_guard = begin_pending_append(&directory, &pending, durable_bytes)?;
    let append = file
        .write_all(encoded)
        .and_then(|()| file.sync_data())
        .and_then(|()| {
            if existed && durable_bytes != 0 {
                Ok(())
            } else {
                directory.sync_all()
            }
        });
    if let Err(error) = append {
        let rollback = file.set_len(durable_bytes).and_then(|()| file.sync_data());
        let message = match rollback {
            Ok(()) => match clear_pending_append(&directory, &pending) {
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
    clear_pending_append(&directory, &pending)?;
    drop(pending_guard);
    Ok(())
}

pub(super) fn scan_entries(
    path: &Path,
    expected_session: SessionId,
    repair_tail: bool,
    after: u64,
    limit: usize,
) -> Result<ScanResult, RepositoryError> {
    super::security::reject_pending_append(path)?;
    let Some(file) = super::security::open_regular_file(path, repair_tail)? else {
        return Ok(ScanResult {
            durable_cutoff: None,
            journal_cutoff: None,
            entries: Vec::new(),
            durable_bytes: 0,
        });
    };

    let mut reader = BufReader::new(file);
    let scan = scan_complete_entries(&mut reader, expected_session, after, limit)?;
    if repair_tail && reader.get_ref().metadata()?.len() > scan.durable_bytes {
        reader.get_mut().set_len(scan.durable_bytes)?;
        reader.get_mut().seek(SeekFrom::Start(scan.durable_bytes))?;
        reader.get_mut().sync_data()?;
    }
    Ok(scan)
}

pub(super) fn scan_complete_entries<R: BufRead>(
    reader: &mut R,
    expected_session: SessionId,
    after: u64,
    limit: usize,
) -> Result<ScanResult, RepositoryError> {
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
            break;
        }
        let wire = WireEntry::decode(&line, line_number)?;
        let expected_sequence = durable_cutoff.map_or(1, |sequence: RepositorySequence| {
            sequence.get().saturating_add(1)
        });
        let decoded = wire.into_record(expected_session, expected_sequence, line_number)?;
        let entry = decoded.entry;
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
        durable_bytes,
    })
}

fn begin_pending_append(
    root: &File,
    pending: &Path,
    durable_bytes: u64,
) -> Result<PendingAppendGuard, RepositoryError> {
    let preparing = pending.with_extension("pending.preparing");
    match statat(root, &preparing, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) if FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile => {
            unlinkat(root, &preparing, AtFlags::empty()).map_err(Error::from)?;
        },
        Ok(_) => {
            return Err(RepositoryError::Unavailable {
                message: format!(
                    "Session append marker preparation path is not a regular file: {}",
                    preparing.display()
                ),
            });
        },
        Err(Errno::NOENT) => {},
        Err(error) => return Err(Error::from(error).into()),
    }
    let mut marker = File::from(
        openat(
            root,
            &preparing,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            super::security::FILE_MODE,
        )
        .map_err(Error::from)
        .map_err(|error| RepositoryError::Unavailable {
            message: format!(
                "failed to prepare the Session append marker at {}: {error}",
                preparing.display()
            ),
        })?,
    );
    super::security::require_user_only_file(&marker)?;
    marker.lock()?;
    writeln!(marker, "{durable_bytes}")?;
    marker.sync_data()?;
    linkat(root, &preparing, root, pending, AtFlags::empty()).map_err(|error| {
        RepositoryError::Quarantined {
            message: format!(
                "failed to publish the Session append marker at {}: {error}",
                pending.display()
            ),
        }
    })?;
    root.sync_all()?;
    unlinkat(root, &preparing, AtFlags::empty()).map_err(Error::from)?;
    Ok(PendingAppendGuard { _file: marker })
}

fn clear_pending_append(root: &File, pending: &Path) -> Result<(), RepositoryError> {
    unlinkat(root, pending, AtFlags::empty()).map_err(Error::from)?;
    // The Session data was already synced. If syncing the marker removal fails,
    // a crash may conservatively restore the marker and quarantine valid data,
    // but it cannot expose a failed append as committed.
    let _ = root.sync_all();
    Ok(())
}
