use std::{collections::HashSet, fmt};

use super::{
    SessionRepository, StoredSessionReader, StoredSessionSnapshot,
    history::normalize_recovered,
    journal::{recover_entries, recover_repository},
};
use crate::{
    AgentCommand, BackendBindingEvidence, BackendIdentity, BackendResumeTarget, SessionDescriptor,
    SessionId, SubmissionId,
    journal::{JournalEntry, codec::JournalRecord},
};

/// Fully validated durable state required to reopen one executable Session.
#[derive(Clone, Debug)]
pub struct StoredSessionContinuation {
    recovered: crate::journal::codec::RecoveredJournal,
    target: BackendResumeTarget,
    next_turn_id: u64,
    transcript_records: Vec<crate::TranscriptRecord>,
}

impl StoredSessionContinuation {
    #[must_use]
    pub const fn descriptor(&self) -> &SessionDescriptor {
        self.recovered
            .descriptor()
            .expect("a continuation always has a descriptor")
    }

    #[must_use]
    pub const fn target(&self) -> &BackendResumeTarget {
        &self.target
    }

    pub(crate) fn semantic_entries(&self) -> Vec<JournalEntry> {
        self.recovered.semantic_entries()
    }

    pub(crate) fn snapshot(&self) -> crate::journal::codec::JournalCommit {
        self.recovered.complete_snapshot()
    }

    pub(crate) fn submission_ids(&self) -> HashSet<SubmissionId> {
        self.recovered.submission_ids().iter().copied().collect()
    }

    pub(crate) const fn next_turn_id(&self) -> u64 {
        self.next_turn_id
    }

    pub(crate) fn transcript_records(&self) -> &[crate::TranscriptRecord] {
        &self.transcript_records
    }
}

/// Why a durable Session cannot be admitted for executable native resume.
#[derive(Debug)]
pub struct StoredSessionContinuationError {
    detail: String,
}

impl StoredSessionContinuationError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for StoredSessionContinuationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for StoredSessionContinuationError {}

/// Revalidates one Session while its repository writer lease is held.
pub fn recover_stored_session_continuation(
    repository: &(impl SessionRepository + ?Sized),
    session_id: SessionId,
) -> Result<StoredSessionContinuation, StoredSessionContinuationError> {
    let recovered = recover_repository(repository, session_id)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    build_continuation(recovered, session_id)
}

/// Validates executable continuation without acquiring a writer lease or mutating storage.
pub fn read_stored_session_continuation(
    reader: &(impl StoredSessionReader + ?Sized),
    session_id: SessionId,
) -> Result<StoredSessionContinuation, StoredSessionContinuationError> {
    let entries = match reader
        .read_session(session_id)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?
    {
        StoredSessionSnapshot::Missing => {
            return Err(StoredSessionContinuationError::new(format!(
                "stored Session {session_id} was not found"
            )));
        },
        StoredSessionSnapshot::Present(entries) if entries.is_empty() => {
            return Err(StoredSessionContinuationError::new(format!(
                "stored Session {session_id} has no complete envelope"
            )));
        },
        StoredSessionSnapshot::Present(entries) => entries,
    };
    let recovered = recover_entries(session_id, &entries)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
    build_continuation(recovered, session_id)
}

fn build_continuation(
    recovered: crate::journal::codec::RecoveredJournal,
    session_id: SessionId,
) -> Result<StoredSessionContinuation, StoredSessionContinuationError> {
    let descriptor = recovered.descriptor().ok_or_else(|| {
        StoredSessionContinuationError::new(format!(
            "stored Session {session_id} has no durable descriptor"
        ))
    })?;
    if descriptor.session_id() != session_id {
        return Err(StoredSessionContinuationError::new(
            "stored Session descriptor identity does not match its repository key",
        ));
    }
    let anchor_sequence = recovered.continuation_anchor().ok_or_else(|| {
        StoredSessionContinuationError::new(format!(
            "stored Session {session_id} has no newest durable Continuation Anchor"
        ))
    })?;
    let epoch = recovered.binding_epoch().ok_or_else(|| {
        StoredSessionContinuationError::new("Continuation Anchor has no open backend binding")
    })?;
    let anchor_epoch = recovered
        .records()
        .iter()
        .find_map(|record| {
            (record.journal_sequence() == Some(anchor_sequence))
                .then(|| match record.record() {
                    JournalRecord::ContinuationAnchor(anchor) => Some(anchor.epoch()),
                    _ => None,
                })
                .flatten()
        })
        .ok_or_else(|| {
            StoredSessionContinuationError::new(format!(
                "Continuation Anchor {} is absent from the recovered semantic Journal",
                anchor_sequence.get()
            ))
        })?;
    if anchor_epoch != epoch {
        return Err(StoredSessionContinuationError::new(format!(
            "Continuation Anchor {} belongs to epoch {anchor_epoch}, not open epoch {epoch}",
            anchor_sequence.get()
        )));
    }

    let mut binding = None;
    let mut max_turn = 0_u64;
    for record in recovered.records() {
        match record.record() {
            JournalRecord::BackendBindingOpened(candidate) if candidate.epoch() == epoch => {
                binding = Some(candidate.clone());
            },
            JournalRecord::CommandCommitted(committed) => match committed.command() {
                AgentCommand::StartTurn { turn, .. }
                | AgentCommand::SteerTurn { turn, .. }
                | AgentCommand::InterruptTurn { turn } => {
                    max_turn = max_turn.max(turn.turn_id().get().get());
                },
                AgentCommand::RespondToActivity { request, .. } => {
                    max_turn = max_turn.max(request.activity().turn().turn_id().get().get());
                },
                AgentCommand::CreateSession { .. } => {},
            },
            _ => {},
        }
    }
    let binding = binding.ok_or_else(|| {
        StoredSessionContinuationError::new(format!(
            "Continuation Anchor {} does not identify a durable backend binding",
            anchor_sequence.get()
        ))
    })?;
    let evidence = BackendBindingEvidence::new(
        binding.backend_kind(),
        binding.backend_version(),
        identity(binding.binding_identity()),
        identity(binding.model_identity()),
        identity(binding.session_locator()),
    );
    let next_turn_id = max_turn.checked_add(1).ok_or_else(|| {
        StoredSessionContinuationError::new("stored Turn identity space is exhausted")
    })?;
    let transcript_records = normalize_recovered(&recovered).map_err(|detail| {
        StoredSessionContinuationError::new(format!(
            "stored Session {session_id} transcript cannot be restored: {detail}"
        ))
    })?;
    Ok(StoredSessionContinuation {
        recovered,
        target: BackendResumeTarget::new(session_id, epoch, evidence),
        next_turn_id,
        transcript_records,
    })
}

fn identity(value: &crate::journal::codec::VersionedIdentity) -> BackendIdentity {
    BackendIdentity::new(value.schema(), value.value())
}

#[cfg(test)]
mod tests;
