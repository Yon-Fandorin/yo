use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use super::{
    error::ConnectionRepositoryError,
    model::ConnectionSnapshot,
    mutation::{ConnectionCommit, ConnectionRepository, PreparedConnectionMutation},
};

mod security;
mod storage;

#[cfg(test)]
pub(super) const PENDING_OPERATION_FILE: &str = security::PENDING_OPERATION_FILE;

#[cfg(test)]
pub(in super::super) use storage::{
    CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST, connection_temporary_path_for_test,
    create_connection_temporary_for_test,
};

/// 정확한 revision CAS를 사용하는 크기 제한 `connections.yaml` local repository입니다.
#[derive(Clone, Debug)]
pub struct LocalConnectionRepository {
    path: PathBuf,
}

impl LocalConnectionRepository {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// connection mutation이 공유하는 process-wide operation lane을 획득합니다.
    pub fn acquire_operation(
        &self,
    ) -> Result<LocalConnectionOperationGuard, ConnectionRepositoryError> {
        let parent = security::prepare_parent(&self.path)?;
        let path = parent.join(security::OPERATION_LOCK_FILE);
        let file = security::open_lock_file(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(LocalConnectionOperationGuard { file, parent }),
            Err(fs::TryLockError::WouldBlock) => {
                Err(ConnectionRepositoryError::OperationBusy(path))
            },
            Err(fs::TryLockError::Error(source)) => {
                Err(ConnectionRepositoryError::io(&path, source))
            },
        }
    }

    /// 더 새로운 operation 구현의 journal을 만나면 fail-closed로 처리합니다.
    pub fn recover_pending_operation(&self) -> Result<(), ConnectionRepositoryError> {
        let Some(parent) = self.path.parent() else {
            return Err(ConnectionRepositoryError::InvalidPath(self.path.clone()));
        };
        let path = parent.join(security::PENDING_OPERATION_FILE);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(ConnectionRepositoryError::PendingOperation(path)),
            Err(source) => Err(ConnectionRepositoryError::io(&path, source)),
        }
    }
}

impl ConnectionRepository for LocalConnectionRepository {
    type OperationGuard = LocalConnectionOperationGuard;

    fn acquire_operation(&self) -> Result<Self::OperationGuard, ConnectionRepositoryError> {
        Self::acquire_operation(self)
    }

    fn recover_pending_operation(&self) -> Result<(), ConnectionRepositoryError> {
        Self::recover_pending_operation(self)
    }

    fn capture(&self) -> Result<ConnectionSnapshot, ConnectionRepositoryError> {
        Self::capture(self)
    }

    fn commit(
        &self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionRepositoryError> {
        Self::commit(self, mutation)
    }
}

#[derive(Debug)]
pub struct LocalConnectionOperationGuard {
    file: fs::File,
    parent: PathBuf,
}

impl LocalConnectionOperationGuard {
    pub(crate) fn authorizes(&self, journal_path: &Path) -> bool {
        journal_path.parent() == Some(self.parent.as_path())
    }
}

impl Drop for LocalConnectionOperationGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
