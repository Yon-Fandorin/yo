use std::{
    fs,
    io::ErrorKind,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::{
    super::LocalConnectionOperationJournal,
    model::{ConnectionOperationExecutionError, ConnectionOperationRepositoryKind},
    paths::{LocalDirectoryIdentity, validate_path_components, validated_parent},
    session::LocalConnectionOperationSession,
};
use crate::model_service::{LocalConnectionRepository, LocalCredentialRepository};

const CONNECTION_FILE: &str = "connections.yaml";
const CREDENTIAL_FILE: &str = "credentials.yaml";
const JOURNAL_FILE: &str = "connection-operation.yaml";

/// 하나의 직렬화된 연결 작업에 참여하는 세 local repository입니다.
///
/// 생성 시 고정된 파일명과 하나의 lexical parent directory를 요구하므로 서로 무관한
/// public, credential, journal 상태를 한 작업에 묶을 수 없습니다.
#[derive(Clone, Debug)]
pub struct LocalConnectionOperationRepositories {
    pub(in super::super) connections: LocalConnectionRepository,
    pub(in super::super) credentials: LocalCredentialRepository,
    pub(in super::super) journal: LocalConnectionOperationJournal,
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
    pub(in super::super) fn acquire_after_lock(
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
                Err(source) if source.kind() == ErrorKind::NotFound => {
                    match fs::create_dir(&current) {
                        Ok(()) => {
                            fs::set_permissions(&current, fs::Permissions::from_mode(0o700))
                                .map_err(|_| invalid(&current))?;
                        },
                        Err(source) if source.kind() == ErrorKind::AlreadyExists => {},
                        Err(_) => return Err(invalid(&current)),
                    }
                },
                Err(_) => return Err(invalid(&current)),
            }
        }
    }
    validate_path_components(ConnectionOperationRepositoryKind::Public, path)
}
