use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::StorageConfigError;

const DEFAULT_CAPACITY_BYTES: u64 = 1024 * 1024 * 1024;

pub(super) fn platform_state_root() -> Result<PathBuf, StorageConfigError> {
    platform_state_root_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

pub(super) fn repository_root_from(
    override_root: Option<OsString>,
    state_root: &Path,
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
        if let Some(state) = xdg_state_home.filter(|state| !state.is_empty()) {
            return Ok(absolute_path("XDG_STATE_HOME", state)?.join("yo"));
        }
        Ok(required_absolute_path_value("HOME", home)?.join(".local/state/yo"))
    }
}

pub(super) fn capacity_bytes() -> Result<u64, StorageConfigError> {
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

#[cfg(test)]
mod tests;
