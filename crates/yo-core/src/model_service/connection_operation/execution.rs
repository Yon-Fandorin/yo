use std::{
    error::Error,
    fmt, fs,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use super::{
    ConnectionCredentialAction, ConnectionOperationError, ConnectionOperationJournalEntry,
    ConnectionOperationKind, ConnectionOperationPhase, ConnectionOperationRecovery,
    ExternalConnectionError, LocalConnectionOperationJournal, plan_connection_recovery,
};
use crate::model_service::{
    ConnectionCommit, ConnectionRepositoryError, ConnectionSnapshot, LocalConnectionOperationGuard,
    LocalConnectionRepository, LocalCredentialRepository, LocalCredentialStoreError,
    PreparedConnectionMutation,
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
    ExternalPreparation(ExternalConnectionError),
    ExternalDisconnectPreparation(super::ExternalDisconnectError),
    PublicCapture(ConnectionRepositoryError),
    PublicPreparation(ConnectionRepositoryError),
    PublicCommit(ConnectionRepositoryError),
    CredentialCapture(LocalCredentialStoreError),
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
    #[cfg(test)]
    InjectedInterruption,
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
            Self::ExternalPreparation(source) => write!(formatter, "{source}"),
            Self::ExternalDisconnectPreparation(source) => write!(formatter, "{source}"),
            Self::PublicCapture(source) => {
                write!(
                    formatter,
                    "capturing public connection state failed: {source}"
                )
            },
            Self::PublicPreparation(source) => {
                write!(
                    formatter,
                    "preparing public connection state failed: {source}"
                )
            },
            Self::PublicCommit(source) => {
                write!(
                    formatter,
                    "committing public connection state failed: {source}"
                )
            },
            Self::CredentialCapture(source) => {
                write!(
                    formatter,
                    "capturing private credential state failed: {source}"
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
            #[cfg(test)]
            Self::InjectedInterruption => {
                formatter.write_str("connection recovery interrupted at an injected test boundary")
            },
        }
    }
}

impl Error for ConnectionOperationExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidRepositoryLayout { .. } => None,
            Self::OperationLock(source) | Self::PublicRepository { source, .. } => Some(source),
            Self::JournalCapture(source) | Self::Journal { source, .. } => Some(source),
            Self::ExternalPreparation(source) => Some(source),
            Self::ExternalDisconnectPreparation(source) => Some(source),
            Self::PublicCapture(source)
            | Self::PublicPreparation(source)
            | Self::PublicCommit(source) => Some(source),
            Self::CredentialCapture(source) => Some(source),
            Self::CredentialRepository { source, .. } => Some(source),
            #[cfg(test)]
            Self::InjectedInterruption => None,
        }
    }
}

/// The three local repositories that participate in one serialized connection operation.
///
/// Construction requires the closed filenames and one lexical parent directory, preventing a
/// caller from pairing a journal with unrelated public or credential state.
#[derive(Clone, Debug)]
pub struct LocalConnectionOperationRepositories {
    pub(super) connections: LocalConnectionRepository,
    pub(super) credentials: LocalCredentialRepository,
    pub(super) journal: LocalConnectionOperationJournal,
    directory: PathBuf,
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
        validate_path_components(ConnectionOperationRepositoryKind::Public, parent)?;
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
            validate_path_components(repository, observed_parent)?;
        }
        let directory = parent.to_owned();
        Ok(Self {
            connections: LocalConnectionRepository::new(connection_path),
            credentials: LocalCredentialRepository::new(credential_path),
            journal: LocalConnectionOperationJournal::new(journal_path),
            directory,
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
        self.acquire_with(|| {})
    }

    fn acquire_with(
        &self,
        after_lock: impl FnOnce(),
    ) -> Result<LocalConnectionOperationSession<'_>, ConnectionOperationExecutionError> {
        prepare_identity_directory(&self.directory)?;
        let directory_identity = LocalDirectoryIdentity::capture(&self.directory)?;
        let guard = self
            .connections
            .acquire_operation()
            .map_err(ConnectionOperationExecutionError::OperationLock)?;
        after_lock();
        directory_identity.revalidate()?;
        Ok(LocalConnectionOperationSession {
            repositories: self,
            guard,
            directory_identity,
        })
    }

    #[cfg(test)]
    pub(super) fn acquire_after_lock(
        &self,
        after_lock: impl FnOnce(),
    ) -> Result<LocalConnectionOperationSession<'_>, ConnectionOperationExecutionError> {
        self.acquire_with(after_lock)
    }
}

fn prepare_identity_directory(path: &Path) -> Result<(), ConnectionOperationExecutionError> {
    validate_path_components(ConnectionOperationRepositoryKind::Public, path)?;
    let invalid = |path: &Path| ConnectionOperationExecutionError::InvalidRepositoryLayout {
        repository: ConnectionOperationRepositoryKind::Public,
        path: path.to_owned(),
    };
    let mut current = PathBuf::from("/");
    for component in path.components().skip(1) {
        current.push(component.as_os_str());
        loop {
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => break,
                Ok(_) => return Err(invalid(&current)),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    match fs::create_dir(&current) {
                        Ok(()) => {
                            fs::set_permissions(&current, fs::Permissions::from_mode(0o700))
                                .map_err(|_| invalid(&current))?;
                        },
                        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {},
                        Err(_) => return Err(invalid(&current)),
                    }
                },
                Err(_) => return Err(invalid(&current)),
            }
        }
    }
    validate_path_components(ConnectionOperationRepositoryKind::Public, path)
}

/// One held local operation lane. The guard remains alive after recovery for subsequent planning.
pub struct LocalConnectionOperationSession<'a> {
    pub(super) repositories: &'a LocalConnectionOperationRepositories,
    pub(super) guard: LocalConnectionOperationGuard,
    pub(super) directory_identity: LocalDirectoryIdentity,
}

impl LocalConnectionOperationSession<'_> {
    pub fn recover_pending_operation(
        &mut self,
    ) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
        self.recover_pending_operation_with(|_| Ok(()))
    }

    /// Captures public state while retaining the same serialized operation lane.
    pub fn capture_connections(
        &self,
    ) -> Result<ConnectionSnapshot, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        self.repositories
            .connections
            .capture()
            .map_err(ConnectionOperationExecutionError::PublicCapture)
    }

    /// Captures private credential state while retaining the same serialized operation lane.
    pub fn capture_credentials(
        &self,
    ) -> Result<crate::model_service::CredentialSnapshot, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        self.repositories
            .credentials
            .capture()
            .map_err(ConnectionOperationExecutionError::CredentialCapture)
    }

    /// Publishes a preference-only or stored public mutation under the held lane.
    pub fn commit_connection_mutation(
        &mut self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        self.repositories
            .connections
            .commit(mutation)
            .map_err(ConnectionOperationExecutionError::PublicCommit)
    }

    fn recover_pending_operation_with(
        &mut self,
        mut observe: impl FnMut(RecoveryStep) -> Result<(), ConnectionOperationExecutionError>,
    ) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
        self.directory_identity.revalidate()?;
        self.repositories
            .journal
            .cleanup_pending_residues(&mut self.guard)
            .map_err(ConnectionOperationExecutionError::JournalCapture)?;
        self.directory_identity.revalidate()?;
        let Some(entry) = self
            .repositories
            .journal
            .capture()
            .map_err(ConnectionOperationExecutionError::JournalCapture)?
        else {
            return Ok(ConnectionOperationExecutionOutcome::NoPendingOperation);
        };
        execute_recovery(
            self.repositories,
            &self.directory_identity,
            &mut self.guard,
            entry,
            &mut observe,
        )
    }

    #[cfg(test)]
    pub(super) fn recover_pending_operation_until(
        &mut self,
        stop: RecoveryStep,
    ) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
        self.recover_pending_operation_with(|step| {
            if step == stop {
                Err(ConnectionOperationExecutionError::InjectedInterruption)
            } else {
                Ok(())
            }
        })
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
    if !path.is_absolute() || !valid_filename {
        return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
            repository,
            path: path.to_owned(),
        });
    }
    Ok(parent)
}

fn validate_path_components(
    repository: ConnectionOperationRepositoryKind,
    path: &Path,
) -> Result<(), ConnectionOperationExecutionError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
            repository,
            path: path.to_owned(),
        });
    }
    let mut current = PathBuf::from("/");
    for component in path.components().skip(1) {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                    repository,
                    path: current,
                });
            },
            Ok(_) => {},
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => {
                return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                    repository,
                    path: current,
                });
            },
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct LocalDirectoryIdentity {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl LocalDirectoryIdentity {
    fn capture(path: &Path) -> Result<Self, ConnectionOperationExecutionError> {
        validate_path_components(ConnectionOperationRepositoryKind::Public, path)?;
        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(path)
            .map_err(
                |_| ConnectionOperationExecutionError::InvalidRepositoryLayout {
                    repository: ConnectionOperationRepositoryKind::Public,
                    path: path.to_owned(),
                },
            )?;
        let metadata = directory.metadata().map_err(|_| {
            ConnectionOperationExecutionError::InvalidRepositoryLayout {
                repository: ConnectionOperationRepositoryKind::Public,
                path: path.to_owned(),
            }
        })?;
        if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                repository: ConnectionOperationRepositoryKind::Public,
                path: path.to_owned(),
            });
        }
        Ok(Self {
            path: path.to_owned(),
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    pub(super) fn revalidate(&self) -> Result<(), ConnectionOperationExecutionError> {
        let observed = Self::capture(&self.path)?;
        if (observed.device, observed.inode) != (self.device, self.inode) {
            return Err(ConnectionOperationExecutionError::InvalidRepositoryLayout {
                repository: ConnectionOperationRepositoryKind::Public,
                path: self.path.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryStep {
    JournalAbandoned,
    JournalAdvanced(ConnectionOperationPhase),
    PublicCommitted,
    CredentialRemoved,
    JournalCleared,
}

fn execute_recovery(
    repositories: &LocalConnectionOperationRepositories,
    directory_identity: &LocalDirectoryIdentity,
    guard: &mut LocalConnectionOperationGuard,
    entry: ConnectionOperationJournalEntry,
    observe: &mut impl FnMut(RecoveryStep) -> Result<(), ConnectionOperationExecutionError>,
) -> Result<ConnectionOperationExecutionOutcome, ConnectionOperationExecutionError> {
    let recovered_from = entry.phase();
    directory_identity.revalidate()?;
    let credentials = repositories
        .credentials
        .capture()
        .map_err(|source| credential_error(&entry, source))?;
    directory_identity.revalidate()?;
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
            directory_identity.revalidate()?;
            repositories
                .journal
                .abandon_intent(guard, &entry)
                .map_err(|source| journal_error(&entry, source))?;
            observe(RecoveryStep::JournalAbandoned)?;
            ConnectionOperationExecutionOutcome::Abandoned { kind, action }
        },
        ConnectionOperationRecovery::CommitPublic => {
            let mut entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::CredentialCommitted,
                directory_identity,
                observe,
            )?;
            directory_identity.revalidate()?;
            repositories
                .connections
                .commit(entry.connection_mutation())
                .map_err(|source| public_error(&entry, source))?;
            observe(RecoveryStep::PublicCommitted)?;
            entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::PublicCommitted,
                directory_identity,
                observe,
            )?;
            complete_and_clear(repositories, directory_identity, guard, entry, observe)?;
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
                directory_identity,
                observe,
            )?;
            let mutation = entry
                .credential_mutation()
                .expect("disconnect-remove journal entries always contain a mutation");
            directory_identity.revalidate()?;
            repositories
                .credentials
                .commit(mutation, None)
                .map_err(|source| credential_error(&entry, source))?;
            observe(RecoveryStep::CredentialRemoved)?;
            entry = advance_to(
                repositories,
                guard,
                entry,
                ConnectionOperationPhase::CredentialRemoved,
                directory_identity,
                observe,
            )?;
            complete_and_clear(repositories, directory_identity, guard, entry, observe)?;
            ConnectionOperationExecutionOutcome::Completed {
                kind,
                action,
                recovered_from,
            }
        },
        ConnectionOperationRecovery::Complete => {
            complete_and_clear(repositories, directory_identity, guard, entry, observe)?;
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
    directory_identity: &LocalDirectoryIdentity,
    observe: &mut impl FnMut(RecoveryStep) -> Result<(), ConnectionOperationExecutionError>,
) -> Result<ConnectionOperationJournalEntry, ConnectionOperationExecutionError> {
    while entry.phase() != target {
        let next = entry
            .next_phase()
            .ok_or_else(|| journal_error(&entry, ConnectionOperationError::InvalidEntry))?;
        directory_identity.revalidate()?;
        entry = repositories
            .journal
            .advance(guard, &entry, next)
            .map_err(|source| journal_error(&entry, source))?;
        observe(RecoveryStep::JournalAdvanced(next))?;
    }
    Ok(entry)
}

fn complete_and_clear(
    repositories: &LocalConnectionOperationRepositories,
    directory_identity: &LocalDirectoryIdentity,
    guard: &mut LocalConnectionOperationGuard,
    entry: ConnectionOperationJournalEntry,
    observe: &mut impl FnMut(RecoveryStep) -> Result<(), ConnectionOperationExecutionError>,
) -> Result<(), ConnectionOperationExecutionError> {
    let entry = advance_to(
        repositories,
        guard,
        entry,
        ConnectionOperationPhase::Complete,
        directory_identity,
        observe,
    )?;
    directory_identity.revalidate()?;
    repositories
        .journal
        .clear_complete(guard, &entry)
        .map_err(|source| journal_error(&entry, source))?;
    observe(RecoveryStep::JournalCleared)
}

pub(super) fn public_error(
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

pub(super) fn credential_error(
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

pub(super) fn journal_error(
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
