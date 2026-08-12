use std::{ffi::OsString, path::PathBuf};

use yo_core::{
    LocalWorkspaceHostIdentity, LocalWorkspaceHostIdentityError, WorkspaceHostId,
    session_repository::{LocalSessionReader, LocalSessionRepository, RepositoryError},
};

const DEFAULT_CAPACITY_BYTES: u64 = 1024 * 1024 * 1024;

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

fn platform_state_root() -> Result<PathBuf, StorageConfigError> {
    platform_state_root_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

fn repository_root_from(
    override_root: Option<OsString>,
    state_root: &std::path::Path,
) -> Result<PathBuf, StorageConfigError> {
    if let Some(root) = override_root {
        return non_empty_path("YO_SESSION_REPOSITORY", root);
    }
    Ok(state_root.join("sessions"))
}

fn platform_state_root_from(
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, StorageConfigError> {
    #[cfg(target_os = "macos")]
    {
        let _ = xdg_state_home;
        let home = required_absolute_path_value("HOME", home)?;
        Ok(home.join("Library").join("Application Support").join("yo"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(state) = xdg_state_home {
            return Ok(absolute_path("XDG_STATE_HOME", state)?.join("yo"));
        }
        Ok(required_absolute_path_value("HOME", home)?.join(".local/state/yo"))
    }
}

fn open_at(
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

fn open_host_identity_at(state_root: PathBuf) -> Result<WorkspaceHostId, StorageConfigError> {
    Ok(LocalWorkspaceHostIdentity::open(state_root.join("host"))
        .map_err(StorageConfigError::HostIdentity)?
        .id())
}

fn open_reader_at(
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

fn capacity_bytes() -> Result<u64, StorageConfigError> {
    capacity_bytes_from(std::env::var_os("YO_SESSION_CAPACITY_BYTES"))
}

fn capacity_bytes_from(value: Option<OsString>) -> Result<u64, StorageConfigError> {
    let Some(value) = value else {
        return Ok(DEFAULT_CAPACITY_BYTES);
    };
    let value = value
        .into_string()
        .map_err(|_| StorageConfigError::InvalidEnvironment {
            name: "YO_SESSION_CAPACITY_BYTES",
            reason: "value is not UTF-8".to_owned(),
        })?;
    value
        .parse::<u64>()
        .map_err(|_| StorageConfigError::InvalidEnvironment {
            name: "YO_SESSION_CAPACITY_BYTES",
            reason: "value must be an unsigned byte count".to_owned(),
        })
}

fn required_absolute_path_value(
    name: &'static str,
    value: Option<OsString>,
) -> Result<PathBuf, StorageConfigError> {
    let value = value.ok_or(StorageConfigError::InvalidEnvironment {
        name,
        reason: "value is not set".to_owned(),
    })?;
    absolute_path(name, value)
}

fn absolute_path(name: &'static str, value: OsString) -> Result<PathBuf, StorageConfigError> {
    let path = non_empty_path(name, value)?;
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(StorageConfigError::InvalidEnvironment {
            name,
            reason: "path is not absolute".to_owned(),
        })
    }
}

fn non_empty_path(name: &'static str, value: OsString) -> Result<PathBuf, StorageConfigError> {
    if value.is_empty() {
        Err(StorageConfigError::InvalidEnvironment {
            name,
            reason: "path is empty".to_owned(),
        })
    } else {
        Ok(PathBuf::from(value))
    }
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

#[cfg(test)]
mod tests;
