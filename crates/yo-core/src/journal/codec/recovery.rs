//! Journal recovery facade and phase ordering.

mod apply;
mod correlation;
mod model;
mod selection;

pub(crate) use model::{HistoricalForkKind, RecoveredJournal};

use super::{JournalCodecError, JournalCommit, JournalCommitKind};

impl RecoveredJournal {
    pub(crate) fn with_incremental(
        &self,
        commit: &JournalCommit,
    ) -> Result<Self, JournalCodecError> {
        if commit.kind() != JournalCommitKind::Incremental {
            return Err(JournalCodecError::new(
                "incremental recovery cannot apply a snapshot",
            ));
        }
        if let (Some(next), Some(current)) = (commit.journal_cutoff(), self.journal_cutoff)
            && next < current
        {
            return Err(JournalCodecError::new(
                "semantic Journal cutoff moved backwards",
            ));
        }
        let mut candidate = self.clone();
        candidate.recovery_commit = None;
        apply::validate_commit_prefix(&candidate, commit)?;
        selection::validate_fork_bootstrap(&candidate, commit)?;
        apply::apply_commit(&mut candidate, commit)?;
        candidate.recovery_commit = apply::recovery_seals(
            candidate.head,
            candidate.journal_cutoff,
            &candidate.open_messages,
        )?;
        Ok(candidate)
    }
}

pub(crate) fn recover(commits: &[JournalCommit]) -> Result<RecoveredJournal, JournalCodecError> {
    commits
        .first()
        .ok_or_else(|| JournalCodecError::new("Journal recovery requires a semantic commit"))?;
    let mut recovered = RecoveredJournal::new();

    for (commit_index, commit) in commits.iter().enumerate() {
        let result = (|| {
            apply::validate_commit_prefix(&recovered, commit)?;
            selection::validate_fork_bootstrap(&recovered, commit)?;
            apply::apply_commit(&mut recovered, commit)
        })();
        result.map_err(|error| error.with_commit_index(commit_index))?;
    }

    recovered.recovery_commit = apply::recovery_seals(
        recovered.head,
        recovered.journal_cutoff,
        &recovered.open_messages,
    )?;
    Ok(recovered)
}
