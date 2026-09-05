use std::{env, ffi::OsString, path::PathBuf};

use super::ConfigError;

pub(super) fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = env::var_os("YO_CONFIG") {
        if path.is_empty() {
            return Err(ConfigError::Environment("YO_CONFIG must not be empty"));
        }
        return Ok(PathBuf::from(path));
    }
    default_config_path()
}

#[cfg(target_os = "macos")]
fn default_config_path() -> Result<PathBuf, ConfigError> {
    let home = env::var_os("HOME").ok_or(ConfigError::Environment(
        "HOME is required to locate Yo configuration",
    ))?;
    Ok(environment_root("HOME", home)?
        .join("Library")
        .join("Application Support")
        .join("yo")
        .join("config.yaml"))
}

#[cfg(not(target_os = "macos"))]
fn default_config_path() -> Result<PathBuf, ConfigError> {
    let root = match env::var_os("XDG_CONFIG_HOME") {
        Some(root) if !root.is_empty() => environment_root("XDG_CONFIG_HOME", root)?,
        _ => {
            let home = env::var_os("HOME").ok_or(ConfigError::Environment(
                "HOME is required when XDG_CONFIG_HOME is not set",
            ))?;
            environment_root("HOME", home)?.join(".config")
        },
    };
    Ok(root.join("yo").join("config.yaml"))
}

pub(super) fn environment_root(
    name: &'static str,
    value: OsString,
) -> Result<PathBuf, ConfigError> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(ConfigError::Environment(match name {
            "HOME" => "HOME must be a non-empty absolute path",
            "XDG_CONFIG_HOME" => "XDG_CONFIG_HOME must be a non-empty absolute path",
            _ => "configuration root must be a non-empty absolute path",
        }));
    }
    Ok(path)
}
