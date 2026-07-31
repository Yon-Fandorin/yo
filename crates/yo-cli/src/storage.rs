use std::{ffi::OsString, path::PathBuf};

use yo_core::session_repository::{LocalSessionRepository, RepositoryError};

const DEFAULT_CAPACITY_BYTES: u64 = 1024 * 1024 * 1024;

pub(crate) fn open_default() -> Result<LocalSessionRepository, StorageConfigError> {
    let root = repository_root()?;
    let capacity = capacity_bytes()?;
    LocalSessionRepository::open(root, capacity).map_err(StorageConfigError::Repository)
}

fn repository_root() -> Result<PathBuf, StorageConfigError> {
    repository_root_from(
        std::env::var_os("YO_SESSION_REPOSITORY"),
        std::env::var_os("XDG_STATE_HOME"),
        std::env::var_os("HOME"),
    )
}

fn repository_root_from(
    override_root: Option<OsString>,
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, StorageConfigError> {
    if let Some(root) = override_root {
        return non_empty_path("YO_SESSION_REPOSITORY", root);
    }

    #[cfg(target_os = "macos")]
    {
        let _ = xdg_state_home;
        let home = required_path_value("HOME", home)?;
        return Ok(home
            .join("Library")
            .join("Application Support")
            .join("yo")
            .join("sessions"));
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(state) = xdg_state_home {
            return Ok(non_empty_path("XDG_STATE_HOME", state)?.join("yo/sessions"));
        }
        Ok(required_path_value("HOME", home)?.join(".local/state/yo/sessions"))
    }
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

fn required_path_value(
    name: &'static str,
    value: Option<OsString>,
) -> Result<PathBuf, StorageConfigError> {
    let value = value.ok_or(StorageConfigError::InvalidEnvironment {
        name,
        reason: "value is not set".to_owned(),
    })?;
    non_empty_path(name, value)
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
    Repository(RepositoryError),
}

impl std::fmt::Display for StorageConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEnvironment { name, reason } => {
                write!(formatter, "invalid {name}: {reason}")
            },
            Self::Repository(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StorageConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidEnvironment { .. } => None,
            Self::Repository(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::{DEFAULT_CAPACITY_BYTES, capacity_bytes_from, repository_root_from};

    // 명시적인 repository override는 OS 기본 위치보다 먼저 선택되어 test와 운영자가
    // 같은 단일 writer root를 의도적으로 지정할 수 있어야 한다.
    #[test]
    fn explicit_repository_root_has_priority() {
        let root = repository_root_from(
            Some(OsString::from("/tmp/yo-explicit")),
            Some(OsString::from("/tmp/xdg")),
            Some(OsString::from("/tmp/home")),
        )
        .unwrap();

        assert_eq!(root, PathBuf::from("/tmp/yo-explicit"));
    }

    // capacity 환경값이 없으면 제품 기본 1 GiB를 사용하고, 숫자가 아닌 값은 조용히
    // fallback하지 않아 사용자가 잘못된 저장 한도를 즉시 알 수 있어야 한다.
    #[test]
    fn capacity_uses_the_default_and_rejects_invalid_input() {
        assert_eq!(capacity_bytes_from(None).unwrap(), DEFAULT_CAPACITY_BYTES);
        assert!(capacity_bytes_from(Some(OsString::from("1GiB"))).is_err());
        assert_eq!(
            capacity_bytes_from(Some(OsString::from("4096"))).unwrap(),
            4096
        );
    }

    #[cfg(not(target_os = "macos"))]
    // Linux에서는 XDG state 위치가 HOME fallback보다 우선해 다른 XDG-aware CLI와
    // 동일한 사용자 상태 디렉터리 규칙을 지킨다.
    #[test]
    fn linux_prefers_xdg_state_home() {
        let root = repository_root_from(
            None,
            Some(OsString::from("/tmp/xdg")),
            Some(OsString::from("/tmp/home")),
        )
        .unwrap();

        assert_eq!(root, PathBuf::from("/tmp/xdg/yo/sessions"));
    }

    #[cfg(target_os = "macos")]
    // macOS에서는 별도 override가 없으면 사용자 Library의 Application Support 아래를
    // 사용해 Session 파일이 일반 문서나 project 디렉터리에 섞이지 않게 한다.
    #[test]
    fn macos_uses_application_support() {
        let root = repository_root_from(None, None, Some(OsString::from("/tmp/home"))).unwrap();

        assert_eq!(
            root,
            PathBuf::from("/tmp/home/Library/Application Support/yo/sessions")
        );
    }
}
