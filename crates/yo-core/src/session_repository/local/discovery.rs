use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs::{self, File},
    io::Error,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
    str::FromStr,
};

use rustix::{
    fs::{AtFlags, Dir, FileType, statat},
    io::Errno,
};

use super::{
    super::{
        RepositoryEntry, RepositoryError, RepositorySequence, SessionTreeLimits,
        SessionTreePlaceholder, StoredSession, StoredSessionReader, StoredSessionSnapshot,
        StoredSessionSummary, StoredSessionTree, StoredSessionUnavailableReason,
        tree::{self, TreeCandidate},
    },
    reader::{
        TreeReadBudget, read_fork_entries, read_snapshot_entries, read_tail_discovery,
        read_tree_entries,
    },
    security::{open_existing_root, pin_reader_root},
};
use crate::{HostWorkspacePath, SessionId, WorkspaceHostId};

#[derive(Debug)]
pub struct LocalSessionReader {
    #[cfg(test)]
    root: PathBuf,
    tree_root: File,
}

impl LocalSessionReader {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        let requested = root.into();
        super::security::validate_repository_root(&requested)?;
        let original = fs::symlink_metadata(&requested)?;
        let root = open_existing_root(&requested)?;
        let tree_root = pin_reader_root(&root)?;
        let pinned = tree_root.metadata()?;
        if original.dev() != pinned.dev() || original.ino() != pinned.ino() {
            return Err(RepositoryError::Unavailable {
                message: "Session repository root changed while the reader was opening".into(),
            });
        }
        Ok(Self {
            #[cfg(test)]
            root,
            tree_root,
        })
    }

    #[cfg(test)]
    pub(in crate::session_repository) fn root_path(&self) -> &Path {
        &self.root
    }

    fn session_path(&self, session_id: SessionId) -> PathBuf {
        PathBuf::from(format!("{session_id}.jsonl"))
    }
}

impl StoredSessionReader for LocalSessionReader {
    fn read_tree(
        &self,
        workspace_host: WorkspaceHostId,
        workspace: &HostWorkspacePath,
        limits: SessionTreeLimits,
    ) -> Result<StoredSessionTree, RepositoryError> {
        let mut paths = BTreeMap::new();
        let mut duplicates = BTreeSet::new();
        let mut truncated = false;
        let directory = Dir::read_from(&self.tree_root).map_err(Error::from)?;
        let mut index = 0;
        for entry in directory {
            let entry = entry.map_err(Error::from)?;
            let name = entry.file_name().to_bytes();
            if matches!(name, b"." | b"..") {
                continue;
            }
            if index == limits.directory_entries() {
                truncated = true;
                break;
            }
            index += 1;
            let path = PathBuf::from(OsStr::from_bytes(name));
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let Ok(session_id) = SessionId::from_str(stem) else {
                continue;
            };
            if paths.insert(session_id, path).is_some() {
                duplicates.insert(session_id);
            }
        }
        let collected: BTreeSet<_> = paths.keys().copied().collect();
        truncated |= paths.len() > limits.sessions();
        let mut candidates = Vec::new();
        let mut budget = TreeReadBudget::new(limits);
        for (session_id, path) in paths.into_iter().take(limits.sessions()) {
            if budget.exhausted() {
                candidates.push(TreeCandidate::uninspected(session_id));
                continue;
            }
            let inspected = if duplicates.contains(&session_id) {
                Err(RepositoryError::Unavailable {
                    message: "multiple repository filenames claim the same Session identity".into(),
                })
            } else {
                read_tree_entries(&self.tree_root, &path, session_id, &mut budget)
            };
            let candidate = match inspected {
                Ok(Some((entries, summary))) => {
                    TreeCandidate::inspect(StoredSession::Available(summary), &entries)
                },
                Ok(None) => TreeCandidate::unavailable(
                    StoredSession::Unavailable {
                        session_id,
                        reason: StoredSessionUnavailableReason::NoCompleteEnvelope,
                    },
                    "Session tree candidate has no complete envelope".into(),
                ),
                Err(_) if budget.exhausted() => {
                    truncated = true;
                    TreeCandidate::uninspected(session_id)
                },
                Err(error) => {
                    let detail = error.to_string();
                    let reason = match error {
                        RepositoryError::Quarantined { message } => {
                            StoredSessionUnavailableReason::Quarantined { message }
                        },
                        RepositoryError::UnsupportedSchema { schema } => {
                            StoredSessionUnavailableReason::UnsupportedSchema { schema }
                        },
                        RepositoryError::CorruptLog { .. }
                        | RepositoryError::CorruptTail { .. } => {
                            StoredSessionUnavailableReason::Corrupt {
                                message: detail.clone(),
                            }
                        },
                        RepositoryError::Unavailable { message } => {
                            StoredSessionUnavailableReason::Unreadable { message }
                        },
                    };
                    TreeCandidate::unavailable(
                        StoredSession::Unavailable { session_id, reason },
                        detail,
                    )
                },
            };
            candidates.push(candidate);
        }
        Ok(tree::assemble(
            candidates,
            workspace_host,
            workspace,
            truncated,
            |parent| {
                if collected.contains(&parent) {
                    return SessionTreePlaceholder::Uninspected;
                }
                match statat(
                    &self.tree_root,
                    format!("{parent}.jsonl"),
                    AtFlags::SYMLINK_NOFOLLOW,
                ) {
                    Err(Errno::NOENT) => SessionTreePlaceholder::MissingAncestor,
                    Ok(metadata)
                        if FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile =>
                    {
                        SessionTreePlaceholder::Uninspected
                    },
                    Ok(_) | Err(_) => SessionTreePlaceholder::Unavailable,
                }
            },
        ))
    }

    fn discover(&self) -> Result<Vec<StoredSession>, RepositoryError> {
        let mut sessions = Vec::new();
        let directory = Dir::read_from(&self.tree_root).map_err(Error::from)?;
        for entry in directory {
            let entry = entry.map_err(Error::from)?;
            let path = PathBuf::from(OsStr::from_bytes(entry.file_name().to_bytes()));
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
            match read_tail_discovery(&self.tree_root, &path, session_id) {
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
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => left.session_id().cmp(&right.session_id()),
        });
        Ok(sessions)
    }

    fn read_session(
        &self,
        session_id: SessionId,
    ) -> Result<StoredSessionSnapshot, RepositoryError> {
        read_snapshot_entries(
            &self.tree_root,
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

    fn read_session_bounded(
        &self,
        session_id: SessionId,
        limits: super::super::SessionForkLimits,
    ) -> Result<StoredSessionSnapshot, RepositoryError> {
        let mut budget = TreeReadBudget::for_fork(limits);
        read_fork_entries(
            &self.tree_root,
            Path::new(&format!("{session_id}.jsonl")),
            session_id,
            &mut budget,
        )
        .map(|captured| {
            captured.map_or(StoredSessionSnapshot::Missing, |(entries, _)| {
                StoredSessionSnapshot::Present(entries)
            })
        })
    }

    fn read_after(
        &self,
        session_id: SessionId,
        sequence: Option<RepositorySequence>,
        limit: usize,
    ) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        read_snapshot_entries(
            &self.tree_root,
            &self.session_path(session_id),
            session_id,
            sequence.map_or(0, RepositorySequence::get),
            limit,
        )
        .map(Option::unwrap_or_default)
    }
}
