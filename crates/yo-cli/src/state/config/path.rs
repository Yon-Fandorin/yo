use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::ConfigError;

pub(super) fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = env::var_os("YO_CONFIG") {
        if path.is_empty() {
            return Err(ConfigError::Environment("YO_CONFIG must not be empty"));
        }
        return absolute_config_path(PathBuf::from(path));
    }
    default_config_path()
}

fn absolute_config_path(path: PathBuf) -> Result<PathBuf, ConfigError> {
    if path.is_absolute() {
        return Ok(path);
    }
    let directory = env::current_dir().map_err(|source| ConfigError::Io {
        path: PathBuf::from("."),
        source,
    })?;
    resolve_from_directory(path, &directory)
}

pub(super) fn resolve_from_directory(
    path: PathBuf,
    directory: &Path,
) -> Result<PathBuf, ConfigError> {
    if path.is_absolute() {
        return Ok(path);
    }
    if !directory.is_absolute() {
        return Err(ConfigError::Environment(
            "the current working directory must be an absolute path",
        ));
    }
    Ok(directory.join(path))
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

fn environment_root(name: &'static str, value: OsString) -> Result<PathBuf, ConfigError> {
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

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::{environment_root, resolve_from_directory};

    // 기본 설정 root는 현재 디렉터리에 따라 뜻이 바뀌는 상대경로를 허용하지 않습니다.
    #[test]
    fn default_configuration_roots_require_absolute_paths() {
        assert!(environment_root("HOME", OsString::from("")).is_err());
        assert!(environment_root("HOME", OsString::from("relative")).is_err());
        assert!(environment_root("XDG_CONFIG_HOME", OsString::from("config")).is_err());
        assert_eq!(
            environment_root("HOME", OsString::from("/home/user")).unwrap(),
            PathBuf::from("/home/user")
        );
    }

    // 상대 YO_CONFIG는 기존 cwd 기준 의미를 유지하되 state 경계에 전달되기 전에
    // 비어 있지 않은 절대 경로로 한 번 고정되는 경계
    #[test]
    fn relative_explicit_configuration_is_fixed_to_the_current_directory() {
        let current = PathBuf::from("/workspace/project");
        assert_eq!(
            resolve_from_directory(PathBuf::from("config.yaml"), &current).unwrap(),
            PathBuf::from("/workspace/project/config.yaml")
        );
        assert!(resolve_from_directory(PathBuf::from("config.yaml"), &PathBuf::from(".")).is_err());
    }
}
