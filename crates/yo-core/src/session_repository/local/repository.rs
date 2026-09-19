#[cfg(test)]
use std::path::Path;
use std::{
    collections::HashMap,
    fs,
    io::Error,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    super::{
        AppendError, AppendReceipt, DurableCutoff, DurableRecord, DurableRecordKind,
        RepositoryEntry, RepositoryError, RepositorySequence, SessionRepository,
        SessionWriterRepository, StoragePressure, StoragePressureCause,
    },
    file::{
        LegacyWriterCompatibilityGuard, RootAppendGuard, SessionWriterLease, append_line,
        coordination_file, process_root_append_coordinator, scan_entries,
    },
    security::prepare_root,
    wire::WireEntry,
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
    session_leases: HashMap<SessionId, SessionWriterLease>,
    root_append_coordinator: Arc<Mutex<()>>,
    _legacy_compatibility_guard: LegacyWriterCompatibilityGuard,
}

impl LocalSessionRepository {
    pub fn open(root: impl Into<PathBuf>, capacity_bytes: u64) -> Result<Self, RepositoryError> {
        let requested = root.into();
        super::security::validate_repository_root(&requested)?;
        let root = prepare_root(&requested)?;
        let legacy_compatibility_guard = LegacyWriterCompatibilityGuard::acquire(&root)?;
        let root_append_coordinator = process_root_append_coordinator(&root);

        Ok(Self {
            root,
            capacity_bytes,
            sessions: HashMap::new(),
            session_leases: HashMap::new(),
            root_append_coordinator,
            _legacy_compatibility_guard: legacy_compatibility_guard,
        })
    }

    pub const fn set_capacity_bytes(&mut self, capacity_bytes: u64) {
        self.capacity_bytes = capacity_bytes;
    }

    #[cfg(test)]
    pub(in crate::session_repository) fn root_path(&self) -> &Path {
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
            // 저장소는 소유자가 중지되기 전에 메모리 내 공백이 없었다고 증명할 수 없습니다.
            // 완전한 스냅샷으로 다시 엽니다.
            snapshot_required: scan.durable_cutoff.is_some(),
            reload_required: false,
        })
    }

    fn ensure_session_state(&mut self, session_id: SessionId) -> Result<(), AppendError> {
        self.acquire_session_writer(session_id)
            .map_err(AppendError::Repository)?;
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
                            super::security::known_cutoff(
                                state.durable_cutoff,
                                state.journal_cutoff,
                            )
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
                if coordination_file(&entry.file_name()) {
                    return Ok(total);
                }
                let metadata = fs::symlink_metadata(entry.path())?;
                if metadata.file_type().is_symlink() {
                    return Err(Error::other("repository contains a symbolic link"));
                }
                if metadata.is_file() {
                    total
                        .checked_add(metadata.len())
                        .ok_or_else(|| Error::other("repository size exceeds u64"))
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
            pressure: StoragePressure::new(
                super::security::known_cutoff(state.durable_cutoff, state.journal_cutoff),
                cause,
            ),
            source,
        }
    }
}
impl SessionWriterRepository for LocalSessionRepository {
    fn acquire_session_writer(&mut self, session_id: SessionId) -> Result<(), RepositoryError> {
        if !self.session_leases.contains_key(&session_id) {
            let lease = SessionWriterLease::acquire(&self.root, session_id)?;
            self.session_leases.insert(session_id, lease);
        }
        Ok(())
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
                durable_cutoff: super::security::known_cutoff(
                    state.durable_cutoff,
                    state.journal_cutoff,
                ),
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

        // macOS에서는 같은 process의 별도 file descriptor가 이 advisory lock 경계에 함께
        // 진입할 수 있으므로 exact root의 in-process coordinator를 먼저 잡습니다.
        // Cross-process serialization은 아래 durable file lock이 유지합니다.
        let root_append_coordinator = Arc::clone(&self.root_append_coordinator);
        let _process_guard = root_append_coordinator
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let append_guard = match RootAppendGuard::acquire(&self.root) {
            Ok(guard) => guard,
            Err(error) => {
                return Err(self.mark_pressure(
                    session_id,
                    StoragePressureCause::Storage,
                    Some(error),
                ));
            },
        };
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
        if storage_bytes.saturating_add(encoded_bytes) > self.capacity_bytes {
            return Err(self.mark_pressure(session_id, StoragePressureCause::Capacity, None));
        }

        let path = self.session_path(session_id);
        if let Err(error) = append_line(&self.root, &path, &encoded) {
            return Err(self.mark_pressure(session_id, StoragePressureCause::Storage, Some(error)));
        }
        drop(append_guard);

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
