use std::{error::Error, fmt, path::PathBuf};

use super::{
    ConnectionCredentialAction, ConnectionOperationError, ConnectionOperationJournalEntry,
    ConnectionOperationKind, ConnectionOperationPhase, ConnectionOperationRecovery,
    LocalConnectionOperationJournal, plan_connection_recovery,
};
use crate::model_service::{
    ConnectionRepositoryError, LocalConnectionOperationGuard, LocalConnectionRepository,
    LocalCredentialRepository, LocalCredentialStoreError,
};

const CONNECTION_FILE: &str = "connections.yaml";
const CREDENTIAL_FILE: &str = "credentials.yaml";
const JOURNAL_FILE: &str = "connection-operation.yaml";

/// The repository role at which local recovery failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionOperationRepositoryKind {
    Public,
    Credential,
    Journal,
}

/// A secret-free result of checking and, when required, completing one pending operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionOperationExecutionOutcome {
    NoPendingOperation,
    Abandoned {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
    },
    Completed {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        recovered_from: ConnectionOperationPhase,
    },
}

/// A typed recovery failure whose diagnostic fields never include a private credential revision.
#[derive(Debug)]
pub enum ConnectionOperationExecutionError {
    InvalidRepositoryLayout {
        repository: ConnectionOperationRepositoryKind,
        path: PathBuf,
    },
    OperationLock(ConnectionRepositoryError),
    JournalCapture(ConnectionOperationError),
    PublicRepository {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        phase: ConnectionOperationPhase,
        source: ConnectionRepositoryError,
    },
    CredentialRepository {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        phase: ConnectionOperationPhase,
        source: LocalCredentialStoreError,
    },
    Journal {
        kind: ConnectionOperationKind,
        action: ConnectionCredentialAction,
        phase: ConnectionOperationPhase,
        source: ConnectionOperationError,
    },
}

impl fmt::Display for ConnectionOperationExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRepositoryLayout { repository, path } => write!(
                formatter,
                "{repository:?} connection-operation repository has an incompatible local path: {}",
                path.display()
            ),
            Self::OperationLock(source) => {
                write!(
                    formatter,
                    "acquiring the connection-operation lock failed: {source}"
                )
            },
            Self::JournalCapture(source) => {
                write!(
                    formatter,
                    "capturing the pending connection operation failed: {source}"
                )
            },
            Self::PublicRepository {
                kind,
                action,
                phase,
                source,
            } => write!(
                formatter,
                "{kind:?} recovery at {phase:?} with {action:?} failed in the public repository: {source}"
            ),
            Self::CredentialRepository {
                kind,
                action,
                phase,
                source,
            } => write!(
                formatter,
                "{kind:?} recovery at {phase:?} with {action:?} failed in the credential repository: {source}"
            ),
            Self::Journal {
                kind,
                action,
                phase,
                source,
            } => write!(
                formatter,
                "{kind:?} recovery at {phase:?} with {action:?} failed in the operation journal: {source}"
            ),
        }
    }
}

impl Error for ConnectionOperationExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidRepositoryLayout { .. } => None,
            Self::OperationLock(source) | Self::PublicRepository { source, .. } => Some(source),
            Self::JournalCapture(source) | Self::Journal { source, .. } => Some(source),
            Self::CredentialRepository { source, .. } => Some(source),
        }
    }
}

/// The three local repositories that participate in one serialized connection operation.
///
/// Construction requires the closed filenames and one lexical parent directory, preventing a
/// caller from pairing a journal with unrelated public or credential state.
#[derive(Clone, Debug)]
pub struct LocalConnectionOperationRepositories {
    connections: LocalConnectionRepository,
    credentials: LocalCredentialRepository,
    journal: LocalConnectionOperationJournal,
}

impl LocalConnectionOperationRepositories {
    pub fn in_directory(
        directory: impl Into<PathBuf>,
    ) -> Result<Self, ConnectionOperationExecutionError> {
        let directory = directory.into();
        Self::from_paths(
            directory.join(CONNECTION_FILE),
            directory.join(CREDENTIAL_FILE),
            directory.join(JOURNAL_FILE),
        )
    }

    pub fn from_paths(
        connection_path: impl Into<PathBuf>,
        credential_path: impl Into<PathBuf>,
        journal_path: impl Into<PathBuf>,
    ) -> Result<Self, ConnectionOperationExecutionError> {
        let connection_path = connection_path.into();
        let credential_path = credential_path.into();
        let journal_path = journal_path.into();
        let parent = validated_parent(
            ConnectionOperationRepositoryKind::Public,
            &connection_path,
            CONNECTION_FILE,
        )?;
        for (repository, path, filename) in [
            (
                ConnectionOperationRepositoryKind::Credential,
                &credential_path,
                CREDENTIAL_FILE,
            ),
            (
                ConnectionOperationRepositoryKind::Journal,
                &journal_path,
                JOURNAL_FILE,
            ),
        ] {
            let observed_parent = validated_parent(repository, path, filename)?;
            if observed_parent != parent {
                return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                    repository,
                    path: path.clone(),
                });
            }
        }
        Ok(Self {
            connections: LocalConnectionRepository::new(connection_path),
            credentials: LocalCredentialRepository::new(credential_path),
            journal: LocalConnectionOperationJournal::new(journal_path),
        })
    }

    #[must_use]
    pub const fn connections(&self) -> &LocalConnectionRepository {
        &self.connections
    }

    #[must_use]
    pub const fn credentials(&self) -> &LocalCredentialRepository {
        &self.credentials
    }

    #[must_use]
    pub const fn journal(&self) -> &LocalConnectionOperationJournal {
        &self.journal
    }

    pub fn acquire(
        &self,
    ) -> Result<LocalConnectionOperationSession<'_>, ConnectionOperationExecutionError> {
        let guard = self
            .connections
            .acquire_operation()
            .map_err(ConnectionOperationExecutionError::OperationLock)?;
        Ok(LocalConnectionOperationSession {
            repositories: self,
            guard,
        })
    }
}

/// One held local operation lane. The guard remains alive after recovery for subsequent planning.
pub struct LocalConnectionOperationSession<'a> {
    repositories: &'a LocalConnectionOperationRepositories,
    guard: LocalConnectionOperationGuard,
}

impl LocalConnectionOperationSession<'_> {
    pub fn recover_pending_operation(
        &mut self,
    ) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
        let Some(entry) = self
            .repositories
            .journal
            .capture()
            .map_err(ConnectionOperationExecutionError::JournalCapture)?
        else {
            return Ok(ConnectionOperationExecutionOutcome::NoPendingOperation);
        };
        execute_recovery(self.repositories, &mut self.guard, entry)
    }
}

fn validated_parent<'a>(
    repository: ConnectionOperationRepositoryKind,
    path: &'a std::path::Path,
    filename: &str,
) -> Result<&'a std::path::Path, ConnectionOperationExecutionError> {
    let valid_filename = path.file_name().is_some_and(|name| name == filename);
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
            repository,
            path: path.to_owned(),
        });
    };
    if !valid_filename {
        return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
            repository,
            path: path.to_owned(),
        });
    }
    Ok(parent)
}

fn execute_recovery(
    repositories: &LocalConnectionOperationRepositories,
    guard: &mut LocalConnectionOperationGuard,
    entry: ConnectionOperationJournalEntry,
) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
    let recovered_from = entry.phase();
    let credentials = repositories
        .credentials
        .capture()
        .map_err(|source| credential_error(&entry, source))?;
    let connections = repositories
        .connections
        .capture()
        .map_err(|source| public_error(&entry, source))?;
    let decision = plan_connection_recovery(&entry, &credentials, &connections)
        .map_err(|source| journal_error(&entry, source))?;
    let kind = entry.kind();
    let action = entry.credential_action();
    let outcome = match decision {
        ConnectionOperationRecovery::Abandon => {
            repositories
                .journal
                .abandon_intent(guard, &entry)
                .map_err(|source| journal_error(&entry, source))?;
            ConnectionOperationExecutionOutcome::Abandoned { kind, action }
        },
        ConnectionOperationRecovery::CommitPublic => {
            let mut entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::CredentialCommitted,
            )?;
            repositories
                .connections
                .commit(entry.connection_mutation())
                .map_err(|source| public_error(&entry, source))?;
            entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::PublicCommitted,
            )?;
            complete_and_clear(repositories, guard, entry)?;
            ConnectionOperationExecutionOutcome::Completed {
                kind,
                action,
                recovered_from,
            }
        },
        ConnectionOperationRecovery::CommitCredentialRemoval => {
            let mut entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::PublicCommitted,
            )?;
            let mutation = entry
                .credential_mutation()
                .expect("disconnect-remove journal entries always contain a mutation");
            repositories
                .credentials
                .commit(mutation, None)
                .map_err(|source| credential_error(&entry, source))?;
            entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::CredentialRemoved,
            )?;
            complete_and_clear(repositories, guard, entry)?;
            ConnectionOperationExecutionOutcome::Completed {
                kind,
                action,
                recovered_from,
            }
        },
        ConnectionOperationRecovery::Complete => {
            complete_and_clear(repositories, guard, entry)?;
            ConnectionOperationExecutionOutcome::Completed {
                kind,
                action,
                recovered_from,
            }
        },
    };
    Ok(outcome)
}

fn advance_to(
    repositories: &LocalConnectionOperationRepositories,
    guard: &mut LocalConnectionOperationGuard,
    mut entry: ConnectionOperationJournalEntry,
    target: ConnectionOperationPhase,
) -> Result<ConnectionOperationJournalEntry, ConnectionOperationExecutionError> {
    while entry.phase() != target {
        let next = entry
            .next_phase()
            .ok_or_else(|| journal_error(&entry, ConnectionOperationError::InvalidEntry))?;
        entry = repositories
            .journal
            .advance(guard, &entry, next)
            .map_err(|source| journal_error(&entry, source))?;
    }
    Ok(entry)
}

fn complete_and_clear(
    repositories: &LocalConnectionOperationRepositories,
    guard: &mut LocalConnectionOperationGuard,
    entry: ConnectionOperationJournalEntry,
) -> Result<(), ConnectionOperationExecutionError> {
    let entry = advance_to(
        repositories,
        guard,
        entry,
        ConnectionOperationPhase::Complete,
    )?;
    repositories
        .journal
        .clear_complete(guard, &entry)
        .map_err(|source| journal_error(&entry, source))
}

fn public_error(
    entry: &ConnectionOperationJournalEntry,
    source: ConnectionRepositoryError,
) -> ConnectionOperationExecutionError {
    ConnectionOperationExecutionError::PublicRepository {
        kind: entry.kind(),
        action: entry.credential_action(),
        phase: entry.phase(),
        source,
    }
}

fn credential_error(
    entry: &ConnectionOperationJournalEntry,
    source: LocalCredentialStoreError,
) -> ConnectionOperationExecutionError {
    ConnectionOperationExecutionError::CredentialRepository {
        kind: entry.kind(),
        action: entry.credential_action(),
        phase: entry.phase(),
        source,
    }
}

fn journal_error(
    entry: &ConnectionOperationJournalEntry,
    source: ConnectionOperationError,
) -> ConnectionOperationExecutionError {
    ConnectionOperationExecutionError::Journal {
        kind: entry.kind(),
        action: entry.credential_action(),
        phase: entry.phase(),
        source,
    }
}
