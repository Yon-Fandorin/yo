mod file;
mod reader;
mod wire;

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use file::{WriterLock, append_line, prepare_root, scan_entries};
use reader::{open_existing_root, read_snapshot_entries, read_tail_discovery};
use wire::WireEntry;

use super::{
    AppendError, AppendReceipt, DurableCutoff, DurableRecord, DurableRecordKind, RepositoryEntry,
    RepositoryError, RepositorySequence, SessionRepository, StoragePressure, StoragePressureCause,
    StoredSession, StoredSessionReader, StoredSessionSnapshot, StoredSessionSummary,
    StoredSessionUnavailableReason,
};
use crate::{JournalSequence, SessionId};

#[derive(Clone, Copy, Debug, Default)]
struct SessionState {
    durable_cutoff: Option<RepositorySequence>,
    journal_cutoff: Option<JournalSequence>,
    cutoff_known: bool,
    snapshot_required: bool,
    reload_required: bool,
}

#[derive(Debug)]
pub struct LocalSessionRepository {
    root: PathBuf,
    capacity_bytes: u64,
    sessions: HashMap<SessionId, SessionState>,
    _writer_lock: WriterLock,
}

#[derive(Debug)]
pub struct LocalSessionReader {
    root: PathBuf,
}

impl LocalSessionReader {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        Ok(Self {
            root: open_existing_root(&root.into())?,
        })
    }

    fn session_path(&self, session_id: SessionId) -> PathBuf {
        self.root.join(format!("{session_id}.jsonl"))
    }
}

impl StoredSessionReader for LocalSessionReader {
    fn discover(&self) -> Result<Vec<StoredSession>, RepositoryError> {
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let Ok(session_id) = SessionId::from_str(stem) else {
                continue;
            };
            match read_tail_discovery(&self.root, &path, session_id) {
                Ok(Some((sequence, version, discovery))) => {
                    sessions.push(StoredSession::Available(StoredSessionSummary::new(
                        sequence, version, discovery,
                    )))
                },
                Ok(None) => sessions.push(StoredSession::Unavailable {
                    session_id,
                    reason: StoredSessionUnavailableReason::NoCompleteEnvelope,
                }),
                Err(RepositoryError::Quarantined { message }) => {
                    sessions.push(StoredSession::Unavailable {
                        session_id,
                        reason: StoredSessionUnavailableReason::Quarantined { message },
                    });
                },
                Err(RepositoryError::UnsupportedSchema { schema }) => {
                    sessions.push(StoredSession::Unavailable {
                        session_id,
                        reason: StoredSessionUnavailableReason::UnsupportedSchema { schema },
                    });
                },
                Err(
                    error @ (RepositoryError::CorruptLog { .. }
                    | RepositoryError::CorruptTail { .. }),
                ) => {
                    sessions.push(StoredSession::Unavailable {
                        session_id,
                        reason: StoredSessionUnavailableReason::Corrupt {
                            message: error.to_string(),
                        },
                    });
                },
                Err(error @ RepositoryError::Unavailable { .. }) => {
                    sessions.push(StoredSession::Unavailable {
                        session_id,
                        reason: StoredSessionUnavailableReason::Unreadable {
                            message: error.to_string(),
                        },
                    });
                },
            }
        }
        sessions.sort_by(|left, right| match (left.summary(), right.summary()) {
            (Some(left), Some(right)) => right
                .discovery()
                .updated_unix_millis()
                .cmp(&left.discovery().updated_unix_millis())
                .then_with(|| {
                    right
                        .discovery()
                        .descriptor()
                        .started_at()
                        .cmp(&left.discovery().descriptor().started_at())
                })
                .then_with(|| {
                    left.discovery()
                        .descriptor()
                        .session_id()
                        .cmp(&right.discovery().descriptor().session_id())
                }),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left.session_id().cmp(&right.session_id()),
        });
        Ok(sessions)
    }

    fn read_session(
        &self,
        session_id: SessionId,
    ) -> Result<StoredSessionSnapshot, RepositoryError> {
        read_snapshot_entries(
            &self.root,
            &self.session_path(session_id),
            session_id,
            0,
            usize::MAX,
        )
        .map(|entries| {
            entries.map_or(
                StoredSessionSnapshot::Missing,
                StoredSessionSnapshot::Present,
            )
        })
    }

    fn read_after(
        &self,
        session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        read_snapshot_entries(
            &self.root,
            &self.session_path(session_id),
            session_id,
            sequence.map_or(0, RepositorySequence::get),
            limit,
        )
        .map(Option::unwrap_or_default)
    }
}

impl LocalSessionRepository {
    pub fn open(root: impl Into<PathBuf>, capacity_bytes: u64) -> Result<Self, RepositoryError> {
        let root = prepare_root(&root.into())?;
        let writer_lock = WriterLock::acquire(&root)?;

        Ok(Self {
            root,
            capacity_bytes,
            sessions: HashMap::new(),
            _writer_lock: writer_lock,
        })
    }

    pub const fn set_capacity_bytes(&mut self, capacity_bytes: u64) {
        self.capacity_bytes = capacity_bytes;
    }

    #[cfg(test)]
    pub(super) fn root_path(&self) -> &std::path::Path {
        &self.root
    }

    fn session_path(&self, session_id: SessionId) -> PathBuf {
        self.root.join(format!("{session_id}.jsonl"))
    }

    fn load_state(&self, session_id: SessionId) -> Result<SessionState, RepositoryError> {
        let scan = scan_entries(&self.session_path(session_id), session_id, true, 0, 0)?;
        Ok(SessionState {
            durable_cutoff: scan.durable_cutoff,
            journal_cutoff: scan.journal_cutoff,
            cutoff_known: true,
            // The repository cannot prove that no in-memory gap occurred
            // before its owner stopped. Reopen through a complete snapshot.
            snapshot_required: scan.durable_cutoff.is_some(),
            reload_required: false,
        })
    }

    fn ensure_session_state(&mut self, session_id: SessionId) -> Result<(), AppendError> {
        let reload_required = self
            .sessions
            .get(&session_id)
            .is_some_and(|state| state.reload_required);
        if self.sessions.contains_key(&session_id) && !reload_required {
            return Ok(());
        }

        match self.load_state(session_id) {
            Ok(mut state) => {
                if reload_required {
                    state.snapshot_required = true;
                }
                self.sessions.insert(session_id, state);
                Ok(())
            },
            Err(
                error @ (RepositoryError::Unavailable { .. } | RepositoryError::Quarantined { .. }),
            ) => {
                let state = self.sessions.entry(session_id).or_default();
                state.snapshot_required = true;
                state.reload_required = true;
                Err(AppendError::StoragePressure {
                    pressure: StoragePressure::new(
                        if state.cutoff_known {
                            known_cutoff(*state)
                        } else {
                            DurableCutoff::Unknown
                        },
                        StoragePressureCause::Storage,
                    ),
                    source: Some(error),
                })
            },
            Err(
                error @ (RepositoryError::CorruptLog { .. }
                | RepositoryError::CorruptTail { .. }
                | RepositoryError::UnsupportedSchema { .. }),
            ) => Err(AppendError::Repository(error)),
        }
    }

    fn storage_bytes(&self) -> Result<u64, RepositoryError> {
        fs::read_dir(&self.root)?
            .try_fold(0_u64, |total, entry| {
                let entry = entry?;
                if entry.file_name() == ".writer.lock"
                    || entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "pending")
                {
                    return Ok(total);
                }
                let metadata = fs::symlink_metadata(entry.path())?;
                if metadata.file_type().is_symlink() {
                    return Err(std::io::Error::other("repository contains a symbolic link"));
                }
                if metadata.is_file() {
                    total
                        .checked_add(metadata.len())
                        .ok_or_else(|| std::io::Error::other("repository size exceeds u64"))
                } else {
                    Ok(total)
                }
            })
            .map_err(RepositoryError::from)
    }

    fn mark_pressure(
        &mut self,
        session_id: SessionId,
        cause: StoragePressureCause,
        source: Option<RepositoryError>,
    ) -> AppendError {
        let state = self
            .sessions
            .get_mut(&session_id)
            .expect("storage pressure is marked only after Session state loads");
        state.snapshot_required = true;
        AppendError::StoragePressure {
            pressure: StoragePressure::new(known_cutoff(*state), cause),
            source,
        }
    }
}

impl SessionRepository for LocalSessionRepository {
    fn append(
        &mut self,
        session_id: SessionId,
        record: DurableRecord,
    ) -> Result<AppendReceipt, AppendError> {
        self.ensure_session_state(session_id)?;

        let state = self
            .sessions
            .get(&session_id)
            .copied()
            .expect("the Session state was inserted above");
        if state.snapshot_required && record.kind() != DurableRecordKind::Snapshot {
            return Err(AppendError::SnapshotRequired {
                durable_cutoff: known_cutoff(state),
            });
        }
        if let (Some(previous), Some(next)) = (state.journal_cutoff, record.journal_cutoff())
            && (next < previous
                || (next == previous && record.kind() != DurableRecordKind::Snapshot))
        {
            return Err(AppendError::Repository(RepositoryError::Unavailable {
                message: "Journal cutoff does not advance for an incremental record".to_owned(),
            }));
        }

        let next = state
            .durable_cutoff
            .map_or(Ok(1), |sequence| sequence.get().checked_add(1).ok_or(()));
        let Ok(next) = next else {
            return Err(AppendError::Repository(RepositoryError::Unavailable {
                message: "Session sequence is exhausted".to_owned(),
            }));
        };
        let sequence = RepositorySequence::new(next);

        let storage_bytes = match self.storage_bytes() {
            Ok(bytes) => bytes,
            Err(error @ RepositoryError::Unavailable { .. }) => {
                return Err(self.mark_pressure(
                    session_id,
                    StoragePressureCause::Storage,
                    Some(error),
                ));
            },
            Err(
                error @ (RepositoryError::Quarantined { .. }
                | RepositoryError::UnsupportedSchema { .. }
                | RepositoryError::CorruptLog { .. }
                | RepositoryError::CorruptTail { .. }),
            ) => {
                return Err(AppendError::Repository(error));
            },
        };
        let remaining = self.capacity_bytes.saturating_sub(storage_bytes);
        if storage_bytes > self.capacity_bytes
            || u64::try_from(record.payload().len()).unwrap_or(u64::MAX) > remaining
        {
            return Err(self.mark_pressure(session_id, StoragePressureCause::Capacity, None));
        }

        let updated_unix_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                AppendError::Repository(RepositoryError::Unavailable {
                    message: format!("failed to timestamp a Session record: {error}"),
                })
            })?
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        let wire = WireEntry::from_record(session_id, sequence, &record, updated_unix_millis)
            .map_err(AppendError::Repository)?;
        let mut encoded = serde_json::to_vec(&wire).map_err(|error| {
            AppendError::Repository(RepositoryError::Unavailable {
                message: format!("failed to encode a Session record: {error}"),
            })
        })?;
        encoded.push(b'\n');
        let encoded_bytes = u64::try_from(encoded.len()).unwrap_or(u64::MAX);
        if storage_bytes.saturating_add(encoded_bytes) > self.capacity_bytes {
            return Err(self.mark_pressure(session_id, StoragePressureCause::Capacity, None));
        }

        let path = self.session_path(session_id);
        if let Err(error) = append_line(&self.root, &path, &encoded) {
            return Err(self.mark_pressure(session_id, StoragePressureCause::Storage, Some(error)));
        }

        let state = self
            .sessions
            .get_mut(&session_id)
            .expect("the Session state was inserted above");
        state.durable_cutoff = Some(sequence);
        if let Some(journal_cutoff) = record.journal_cutoff() {
            state.journal_cutoff = Some(journal_cutoff);
        }
        state.cutoff_known = true;
        state.snapshot_required = false;
        Ok(AppendReceipt::new(sequence))
    }

    fn read_after(
        &self,
        session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let after = sequence.map_or(0, RepositorySequence::get);
        Ok(scan_entries(
            &self.session_path(session_id),
            session_id,
            false,
            after,
            limit,
        )?
        .entries)
    }
}

fn known_cutoff(state: SessionState) -> DurableCutoff {
    match state.durable_cutoff {
        Some(repository_sequence) => DurableCutoff::Known {
            journal_sequence: state.journal_cutoff,
            repository_sequence,
        },
        None => DurableCutoff::KnownEmpty,
    }
}
