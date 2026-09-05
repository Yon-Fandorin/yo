use yo_core::{
    LocalWorkspaceHostIdentityError, WorkspaceHostId, session_repository::RepositoryError,
};

mod environment;
mod repository;

use environment::{capacity_bytes, platform_state_root, repository_root_from};
pub(crate) use repository::{LocalReadStorage, LocalStorage};
use repository::{open_at, open_host_identity_at, open_reader_at};

#[cfg(test)]
mod tests;

pub(crate) fn open_default() -> Result<LocalStorage, StorageConfigError> {
    let state_root = platform_state_root()?;
    let repository_root =
        repository_root_from(std::env::var_os("YO_SESSION_REPOSITORY"), &state_root)?;
    let capacity = capacity_bytes()?;
    open_at(state_root, repository_root, capacity)
}

pub(crate) fn open_default_reader() -> Result<LocalReadStorage, StorageConfigError> {
    let state_root = platform_state_root()?;
    let repository_root =
        repository_root_from(std::env::var_os("YO_SESSION_REPOSITORY"), &state_root)?;
    open_reader_at(state_root, repository_root)
}

pub(crate) fn open_default_host_identity() -> Result<WorkspaceHostId, StorageConfigError> {
    open_host_identity_at(platform_state_root()?)
}

#[derive(Debug)]
pub(crate) enum StorageConfigError {
    InvalidEnvironment { name: &'static str, reason: String },
    HostIdentity(LocalWorkspaceHostIdentityError),
    Repository(RepositoryError),
}

impl std::fmt::Display for StorageConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEnvironment { name, reason } => {
                write!(formatter, "invalid {name}: {reason}")
            },
            Self::HostIdentity(error) => error.fmt(formatter),
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StorageConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidEnvironment { .. } => None,
            Self::HostIdentity(error) => Some(error),
            Self::Repository(error) => Some(error),
        }
    }
}
