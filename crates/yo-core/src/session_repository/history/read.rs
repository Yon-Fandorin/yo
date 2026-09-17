//! durable Session record를 읽고 semantic recovery projection을 조립합니다.

use std::{error::Error, fmt};

use super::{
    discovery::validate_discovery,
    inherited::project_inherited,
    model::{StoredSessionContinuity, StoredSessionHistory, StoredSessionRecovery},
    normalizer::normalize,
    request_trace,
};
use crate::{
    SessionId, interview,
    journal::{SemanticRecord, codec::RecoveredJournal},
    session_repository::{
        RepositoryError, StoredSessionReader, StoredSessionSnapshot, journal::recover_entries,
    },
};

pub(super) fn normalize_recovered(
    recovered: &RecoveredJournal,
) -> Result<Vec<crate::TranscriptRecord>, String> {
    normalize(recovered)
}

/// writer lease를 획득하거나 physical record를 노출하지 않고 durable Session을 복구합니다.
pub fn read_stored_session(
    reader: &(impl StoredSessionReader + ?Sized),
    session_id: SessionId,
) -> Result<StoredSessionHistory, StoredSessionReadError> {
    let entries = match reader
        .read_session(session_id)
        .map_err(StoredSessionReadError::Repository)?
    {
        StoredSessionSnapshot::Missing => {
            return Err(StoredSessionReadError::NotFound { session_id });
        },
        StoredSessionSnapshot::Present(entries) if entries.is_empty() => {
            return Err(StoredSessionReadError::Incomplete { session_id });
        },
        StoredSessionSnapshot::Present(entries) => entries,
    };
    let recovered =
        recover_entries(session_id, &entries).map_err(|error| invalid_stored(error.to_string()))?;
    let descriptor = recovered
        .descriptor()
        .cloned()
        .ok_or_else(|| invalid_stored(format!("stored Session {session_id} has no descriptor")))?;
    let discovery_validation = validate_discovery(&entries, &descriptor, &recovered);
    let recovery = if recovered.recovery_commit().is_some() {
        StoredSessionRecovery::Interrupted
    } else {
        StoredSessionRecovery::NotRequired
    };
    let records = normalize(&recovered).map_err(invalid_stored)?;
    let request_trace = request_trace::project(&recovered);
    let inherited_history = project_inherited(&recovered).map_err(invalid_stored)?;
    let semantic = recovered.semantic_entries();
    let accepted_initial = semantic
        .iter()
        .find_map(|entry| match entry.record() {
            SemanticRecord::CommandCommitted(command)
                if matches!(command.command(), crate::AgentCommand::StartTurn { .. }) =>
            {
                command.submission_id()
            },
            _ => None,
        })
        .and_then(|id| {
            interview::initial_submission_evidence(
                &semantic,
                id,
                crate::JournalDurability::Durable {
                    journal_sequence: recovered.journal_cutoff(),
                    repository_sequence: entries
                        .last()
                        .expect("recovered descriptor exists")
                        .sequence(),
                },
            )
            .map(|(turn, sequence)| (id, turn, sequence))
        });
    Ok(StoredSessionHistory {
        descriptor,
        journal_cutoff: recovered.journal_cutoff(),
        recovery,
        continuity: StoredSessionContinuity::NotObservable,
        discovery_validation,
        records,
        request_trace,
        inherited_history,
        accepted_initial,
    })
}

/// stored Session history를 검증하고 복구하지 못한 이유입니다.
#[derive(Debug)]
pub enum StoredSessionReadError {
    NotFound { session_id: SessionId },
    Incomplete { session_id: SessionId },
    Repository(RepositoryError),
    Invalid { detail: String },
}

impl fmt::Display for StoredSessionReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { session_id } => {
                write!(formatter, "stored Session {session_id} was not found")
            },
            Self::Incomplete { session_id } => {
                write!(
                    formatter,
                    "stored Session {session_id} has no complete envelope"
                )
            },
            Self::Repository(error) => error.fmt(formatter),
            Self::Invalid { detail } => formatter.write_str(detail),
        }
    }
}

impl Error for StoredSessionReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            Self::NotFound { .. } | Self::Incomplete { .. } | Self::Invalid { .. } => None,
        }
    }
}

fn invalid_stored(detail: impl Into<String>) -> StoredSessionReadError {
    StoredSessionReadError::Invalid {
        detail: detail.into(),
    }
}
