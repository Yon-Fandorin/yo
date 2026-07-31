use std::{collections::HashSet, fmt};

use super::{
    AppendError, AppendReceipt, DurableRecord, DurableRecordKind, RepositoryError,
    RepositorySequence, SessionRepository,
};
use crate::{
    SessionId,
    journal::codec::{
        JournalCodecError, JournalCommit, JournalCommitKind, RecoveredJournal, decode, encode,
        recover,
    },
};

#[derive(Debug)]
pub(crate) struct JournalRepository<R> {
    repository: R,
    live_gaps: HashSet<SessionId>,
    loaded_sessions: HashSet<SessionId>,
    recovered: std::collections::HashMap<SessionId, RecoveredJournal>,
}

impl<R> JournalRepository<R>
where
    R: SessionRepository,
{
    pub(crate) fn new(repository: R) -> Self {
        Self {
            repository,
            live_gaps: HashSet::new(),
            loaded_sessions: HashSet::new(),
            recovered: std::collections::HashMap::new(),
        }
    }

    pub(crate) fn append(
        &mut self,
        session_id: SessionId,
        commit: &JournalCommit,
    ) -> Result<AppendReceipt, JournalRepositoryError> {
        require_session(commit, session_id).map_err(JournalRepositoryError::Codec)?;
        let payload = encode(commit).map_err(JournalRepositoryError::Codec)?;
        self.ensure_loaded(session_id)?;
        if commit.kind() == JournalCommitKind::Snapshot {
            let recovered_snapshot;
            let required_prefix = if self.live_gaps.contains(&session_id) {
                self.recovered
                    .get(&session_id)
                    .map_or(&[][..], RecoveredJournal::records)
            } else {
                recovered_snapshot = self
                    .recovered
                    .get(&session_id)
                    .map(RecoveredJournal::complete_snapshot);
                recovered_snapshot
                    .as_ref()
                    .map_or(&[][..], JournalCommit::records)
            };
            if !commit.records().starts_with(required_prefix) {
                return Err(JournalRepositoryError::Codec(
                    JournalCodecError::new(
                        "a complete snapshot must preserve the durable semantic prefix and its recovery seals",
                    )
                    .context("candidate semantic commit"),
                ));
            }
        }
        let replacement = if commit.kind() == JournalCommitKind::Snapshot
            || !self.recovered.contains_key(&session_id)
        {
            Some(recover(std::slice::from_ref(commit)).map_err(|error| {
                JournalRepositoryError::Codec(error.context("candidate semantic commit"))
            })?)
        } else {
            self.recovered[&session_id]
                .validate_incremental(commit)
                .map_err(|error| {
                    JournalRepositoryError::Codec(error.context("candidate semantic commit"))
                })?;
            None
        };
        let record = match commit.kind() {
            JournalCommitKind::Incremental => DurableRecord::incremental(payload),
            JournalCommitKind::Snapshot => DurableRecord::snapshot(payload),
        }
        .with_journal_cutoff(commit.journal_cutoff());
        match self.repository.append(session_id, record) {
            Ok(receipt) => {
                if let Some(replacement) = replacement {
                    self.recovered.insert(session_id, replacement);
                } else {
                    self.recovered
                        .get_mut(&session_id)
                        .expect("the incremental prefix was validated above")
                        .append_validated(commit);
                }
                if commit.kind() == JournalCommitKind::Snapshot {
                    self.live_gaps.remove(&session_id);
                }
                Ok(receipt)
            },
            Err(error) => {
                if matches!(error, AppendError::StoragePressure { .. }) {
                    self.live_gaps.insert(session_id);
                }
                Err(JournalRepositoryError::Append(error))
            },
        }
    }

    #[allow(
        dead_code,
        reason = "stored Session opening consumes recovery in the next approved Slice"
    )]
    pub(crate) fn recover(
        &self,
        session_id: SessionId,
    ) -> Result<RecoveredJournal, JournalRepositoryError> {
        let (commits, origins) = self.load_commits(session_id)?;
        recover(&commits).map_err(|error| {
            let context = recovery_context(&error, &origins);
            JournalRepositoryError::Codec(error.context(context))
        })
    }

    fn load_commits(
        &self,
        session_id: SessionId,
    ) -> Result<(Vec<JournalCommit>, Vec<RepositorySequence>), JournalRepositoryError> {
        let mut after = None;
        let mut commits = Vec::new();
        let mut origins = Vec::new();
        loop {
            let entries = self
                .repository
                .read_after(session_id, after, 256)
                .map_err(JournalRepositoryError::Repository)?;
            if entries.is_empty() {
                break;
            }
            for entry in &entries {
                let physical_sequence = entry.sequence();
                let commit = decode(entry.record().payload())
                    .and_then(|commit| {
                        require_session(&commit, session_id)?;
                        Ok(commit)
                    })
                    .map_err(|error| {
                        JournalRepositoryError::Codec(error.context(format_args!(
                            "repository sequence {}",
                            physical_sequence.get()
                        )))
                    })?;
                let expected_kind = match commit.kind() {
                    JournalCommitKind::Incremental => DurableRecordKind::Incremental,
                    JournalCommitKind::Snapshot => DurableRecordKind::Snapshot,
                };
                if entry.record().kind() != expected_kind
                    || entry.record().journal_cutoff() != commit.journal_cutoff()
                {
                    return Err(JournalRepositoryError::Codec(
                        JournalCodecError::new(
                            "repository envelope does not match its semantic Journal commit",
                        )
                        .context(format_args!(
                            "repository sequence {}",
                            physical_sequence.get()
                        )),
                    ));
                }
                commits.push(commit);
                origins.push(physical_sequence);
            }
            after = entries.last().map(|entry| entry.sequence());
            if entries.len() < 256 {
                break;
            }
        }
        Ok((commits, origins))
    }

    fn ensure_loaded(&mut self, session_id: SessionId) -> Result<(), JournalRepositoryError> {
        if self.loaded_sessions.contains(&session_id) {
            return Ok(());
        }
        let (commits, origins) = self.load_commits(session_id)?;
        if !commits.is_empty() {
            let recovered = recover(&commits).map_err(|error| {
                let context = recovery_context(&error, &origins);
                JournalRepositoryError::Codec(error.context(context))
            })?;
            self.recovered.insert(session_id, recovered);
        }
        self.loaded_sessions.insert(session_id);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn into_inner(self) -> R {
        self.repository
    }
}

fn recovery_context(error: &JournalCodecError, origins: &[RepositorySequence]) -> String {
    error
        .commit_index()
        .and_then(|index| origins.get(index))
        .map_or_else(
            || "semantic Journal recovery".to_owned(),
            |sequence| format!("repository sequence {}", sequence.get()),
        )
}

fn require_session(
    commit: &JournalCommit,
    expected_session: SessionId,
) -> Result<(), JournalCodecError> {
    if commit.session_id() == Some(expected_session) {
        Ok(())
    } else {
        Err(JournalCodecError::new(format!(
            "semantic Journal commit does not belong to Session {}",
            expected_session.get()
        )))
    }
}

#[derive(Debug)]
pub(crate) enum JournalRepositoryError {
    Codec(JournalCodecError),
    Append(AppendError),
    Repository(RepositoryError),
}

impl fmt::Display for JournalRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => error.fmt(formatter),
            Self::Append(error) => error.fmt(formatter),
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for JournalRepositoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            Self::Append(error) => Some(error),
            Self::Repository(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests;
