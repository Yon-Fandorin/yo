use std::{collections::HashSet, fmt};

use super::{
    SessionWriterRepository, StoredSessionReader, StoredSessionSnapshot,
    history::normalize_recovered,
    journal::{recover_entries, recover_repository},
};
use crate::{
    AgentCommand, BackendBindingEvidence, BackendIdentity, BackendResumeSource,
    BackendResumeTarget, ContinuationStrategy, ModelReplay, SessionDescriptor, SessionId,
    SubmissionId,
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

/// Acquires one Session writer lease, then revalidates its executable continuation.
pub fn recover_stored_session_continuation(
    repository: &mut (impl SessionWriterRepository + ?Sized),
    session_id: SessionId,
) -> Result<StoredSessionContinuation, StoredSessionContinuationError> {
    repository
        .acquire_session_writer(session_id)
        .map_err(|error| StoredSessionContinuationError::new(error.to_string()))?;
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

pub(crate) fn build_continuation(
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
    let epoch = recovered.binding_epoch().ok_or_else(|| {
        StoredSessionContinuationError::new("Continuation Anchor has no open backend binding")
    })?;
    let binding = recovered
        .records()
        .iter()
        .find_map(|record| match record.record() {
            JournalRecord::BackendBindingOpened(candidate) if candidate.epoch() == epoch => {
                Some(candidate.clone())
            },
            _ => None,
        })
        .ok_or_else(|| {
            StoredSessionContinuationError::new(
                "the open backend binding is absent from the recovered semantic Journal",
            )
        })?;
    let transition_source = binding
        .transition()
        .source_anchor_sequence()
        .map(BackendResumeSource::ContinuationAnchor)
        .or_else(|| {
            binding
                .transition()
                .source_checkpoint_sequence()
                .map(BackendResumeSource::ContextCheckpoint)
        });
    let open_epoch_has_accepted_request = recovered.records().iter().any(|record| {
        matches!(
            record.record(),
            JournalRecord::BackendRequestAccepted(request) if request.epoch() == epoch
        )
    });
    let resume_source = recovered
        .continuation_anchor()
        .map(BackendResumeSource::ContinuationAnchor)
        .or_else(|| {
            recovered
                .context_checkpoint()
                .map(BackendResumeSource::ContextCheckpoint)
        })
        .or_else(|| {
            (!open_epoch_has_accepted_request)
                .then_some(transition_source)
                .flatten()
        })
        .ok_or_else(|| {
            StoredSessionContinuationError::new(format!(
                "stored Session {session_id} has no newest durable Continuation Anchor or context checkpoint"
            ))
        })?;
    let source_sequence = resume_source.sequence();
    let source_epoch = recovered
        .records()
        .iter()
        .find_map(|record| {
            (record.journal_sequence() == Some(source_sequence)).then_some(record.record())
        })
        .and_then(|record| match (resume_source, record) {
            (
                BackendResumeSource::ContinuationAnchor(_),
                JournalRecord::ContinuationAnchor(anchor),
            ) => Some(anchor.epoch()),
            (
                BackendResumeSource::ContextCheckpoint(_),
                JournalRecord::ContextCheckpoint(checkpoint),
            ) => Some(checkpoint.epoch()),
            _ => None,
        })
        .ok_or_else(|| {
            StoredSessionContinuationError::new(format!(
                "continuation source {} is absent from the recovered semantic Journal",
                source_sequence.get()
            ))
        })?;
    let resumes_from_replacement_source = transition_source == Some(resume_source)
        && binding.transition().mode() == crate::journal::codec::TransitionMode::ExactReplay;
    if source_epoch != epoch && !resumes_from_replacement_source {
        return Err(StoredSessionContinuationError::new(format!(
            "continuation source {} belongs to epoch {source_epoch}, not open epoch {epoch}",
            source_sequence.get()
        )));
    }

    let mut max_turn = 0_u64;
    for record in recovered.records() {
        if let JournalRecord::CommandCommitted(committed) = record.record() {
            match committed.command() {
                AgentCommand::StartTurn { turn, .. }
                | AgentCommand::SteerTurn { turn, .. }
                | AgentCommand::InterruptTurn { turn } => {
                    max_turn = max_turn.max(turn.turn_id().get().get());
                },
                AgentCommand::RespondToActivity { request, .. } => {
                    max_turn = max_turn.max(request.activity().turn().turn_id().get().get());
                },
                AgentCommand::CreateSession { .. } | AgentCommand::CompactContext { .. } => {},
            }
        }
    }
    let evidence = BackendBindingEvidence::new(
        binding.backend_kind(),
        binding.backend_version(),
        identity(binding.binding_identity()),
        identity(binding.model_identity()),
        identity(binding.session_locator()),
        binding.continuation_strategy(),
    );
    let model_replay = match binding.continuation_strategy() {
        ContinuationStrategy::ExactReplay { .. } => recovered.model_replay().clone(),
        ContinuationStrategy::BackendManagedState => ModelReplay::default(),
    };
    let next_turn_id = max_turn.checked_add(1).ok_or_else(|| {
        StoredSessionContinuationError::new("stored Turn identity space is exhausted")
    })?;
    let transcript_records = normalize_recovered(&recovered).map_err(|detail| {
        StoredSessionContinuationError::new(format!(
            "stored Session {session_id} transcript cannot be restored: {detail}"
        ))
    })?;
    let target = match resume_source {
        BackendResumeSource::ContinuationAnchor(sequence) => {
            BackendResumeTarget::new(session_id, epoch, evidence, sequence)
        },
        BackendResumeSource::ContextCheckpoint(sequence) => {
            BackendResumeTarget::from_checkpoint(session_id, epoch, evidence, sequence)
        },
    }
    .with_model_replay(model_replay)
    .with_context_state(
        recovered.context_policy().cloned(),
        recovered.context_epoch(),
        recovered.model_replay_groups(),
    )
    .with_replay_contract_rebind_required(recovered.replay_contract_rebind_required());
    Ok(StoredSessionContinuation {
        recovered,
        target,
        next_turn_id,
        transcript_records,
    })
}

fn identity(value: &crate::journal::codec::VersionedIdentity) -> BackendIdentity {
    BackendIdentity::new(value.schema(), value.value())
}

#[cfg(test)]
mod tests;
