use std::fmt;

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
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "runtime repository ownership is explicitly outside this Slice"
    )
)]
pub(crate) struct JournalRepository<R> {
    repository: R,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "runtime repository ownership is explicitly outside this Slice"
    )
)]
impl<R> JournalRepository<R>
where
    R: SessionRepository,
{
    pub(crate) const fn new(repository: R) -> Self {
        Self { repository }
    }

    pub(crate) fn append(
        &mut self,
        session_id: SessionId,
        commit: &JournalCommit,
    ) -> Result<AppendReceipt, JournalRepositoryError> {
        require_session(commit, session_id).map_err(JournalRepositoryError::Codec)?;
        let payload = encode(commit).map_err(JournalRepositoryError::Codec)?;
        let (mut commits, origins) = self.load_commits(session_id)?;
        if commit.kind() == JournalCommitKind::Snapshot {
            let recovered_prefix = recover(&commits).map_err(|error| {
                let context = recovery_context(&error, &origins);
                JournalRepositoryError::Codec(error.context(context))
            })?;
            let required_prefix = recovered_prefix.complete_snapshot();
            if !commit.records().starts_with(required_prefix.records()) {
                return Err(JournalRepositoryError::Codec(
                    JournalCodecError::new(
                        "a complete snapshot must preserve the durable semantic prefix and its recovery seals",
                    )
                    .context("candidate semantic commit"),
                ));
            }
        }
        let candidate_index = commits.len();
        commits.push(commit.clone());
        recover(&commits).map_err(|error| {
            let context = if error.commit_index() == Some(candidate_index) {
                "candidate semantic commit".to_owned()
            } else {
                recovery_context(&error, &origins)
            };
            JournalRepositoryError::Codec(error.context(context))
        })?;
        let record = match commit.kind() {
            JournalCommitKind::Incremental => DurableRecord::incremental(payload),
            JournalCommitKind::Snapshot => DurableRecord::snapshot(payload),
        }
        .with_journal_cutoff(commit.journal_cutoff());
        self.repository
            .append(session_id, record)
            .map_err(JournalRepositoryError::Append)
    }

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
