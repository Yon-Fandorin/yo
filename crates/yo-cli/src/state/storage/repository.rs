use std::path::PathBuf;

use yo_core::{
    LocalWorkspaceHostIdentity, WorkspaceHostId,
    session_repository::{LocalSessionReader, LocalSessionRepository},
};

use super::StorageConfigError;

pub(crate) struct LocalStorage {
    repository: LocalSessionRepository,
    workspace_host_id: WorkspaceHostId,
}

pub(crate) struct LocalReadStorage {
    reader: Option<LocalSessionReader>,
    workspace_host_id: Option<WorkspaceHostId>,
}

impl LocalReadStorage {
    pub(crate) const fn reader(&self) -> Option<&LocalSessionReader> {
        self.reader.as_ref()
    }

    pub(crate) const fn workspace_host_id(&self) -> Option<WorkspaceHostId> {
        self.workspace_host_id
    }
}

impl LocalStorage {
    pub(crate) fn into_parts(self) -> (LocalSessionRepository, WorkspaceHostId) {
        (self.repository, self.workspace_host_id)
    }
}

pub(super) fn open_at(
    state_root: PathBuf,
    repository_root: PathBuf,
    capacity: u64,
) -> Result<LocalStorage, StorageConfigError> {
    let workspace_host_id = LocalWorkspaceHostIdentity::open(state_root.join("host"))
        .map_err(StorageConfigError::HostIdentity)?
        .id();
    let repository = LocalSessionRepository::open(repository_root, capacity)
        .map_err(StorageConfigError::Repository)?;
    Ok(LocalStorage {
        repository,
        workspace_host_id,
    })
}

pub(super) fn open_host_identity_at(
    state_root: PathBuf,
) -> Result<WorkspaceHostId, StorageConfigError> {
    Ok(LocalWorkspaceHostIdentity::open(state_root.join("host"))
        .map_err(StorageConfigError::HostIdentity)?
        .id())
}

pub(super) fn open_reader_at(
    state_root: PathBuf,
    repository_root: PathBuf,
) -> Result<LocalReadStorage, StorageConfigError> {
    let workspace_host_id = LocalWorkspaceHostIdentity::open_existing(state_root.join("host"))
        .map_err(StorageConfigError::HostIdentity)?
        .map(LocalWorkspaceHostIdentity::id);
    let reader = match std::fs::symlink_metadata(&repository_root) {
        Ok(_) => Some(
            LocalSessionReader::open(repository_root).map_err(StorageConfigError::Repository)?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(StorageConfigError::Repository(error.into())),
    };
    Ok(LocalReadStorage {
        reader,
        workspace_host_id,
    })
}
