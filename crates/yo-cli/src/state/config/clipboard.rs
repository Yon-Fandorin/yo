//! Explicit clipboard source syntax and structural admission, without host I/O.

use std::{
    num::NonZeroU16,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Deserializer};

use crate::state::config::ConfigError;

/// Fixed native reader on the selected clipboard host.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ClipboardReader {
    /// macOS PNG clipboard reader.
    Macos,
    /// Wayland PNG clipboard reader.
    Wayland,
    /// X11 PNG clipboard reader.
    X11,
}

/// Structurally admitted acquisition target, used only after an attachment gesture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClipboardSource {
    /// Clipboard of the process's native desktop.
    Native,
    /// Explicit local or forwarded Unix socket.
    Socket {
        /// Absolute socket path; runtime verifies ownership and permissions.
        path: PathBuf,
    },
    /// Fixed clipboard reader over an explicit SSH target.
    Ssh {
        /// SSH alias, hostname, or user-qualified host.
        host: String,
        /// Reader chosen for the target's desktop.
        reader: ClipboardReader,
        /// Optional absolute SSH identity path.
        identity_file: Option<PathBuf>,
        /// Optional absolute SSH known-hosts path.
        known_hosts_file: Option<PathBuf>,
        /// Optional explicit SSH port.
        port: Option<NonZeroU16>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "source", rename_all = "lowercase", deny_unknown_fields)]
pub(super) enum ClipboardConfig {
    Native {},
    Socket {
        path: String,
    },
    Ssh {
        host: String,
        reader: ClipboardReader,
        identity_file: Option<String>,
        known_hosts_file: Option<String>,
        port: Option<NonZeroU16>,
    },
}

pub(super) fn deserialize_clipboard<'de, D>(
    deserializer: D,
) -> Result<Option<ClipboardConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    ClipboardConfig::deserialize(deserializer).map(Some)
}

impl ClipboardConfig {
    pub(super) fn admit(self, config_path: &Path) -> Result<ClipboardSource, ConfigError> {
        let invalid = |detail| ConfigError::InvalidClipboard {
            path: config_path.to_owned(),
            detail,
        };
        match self {
            Self::Native {} => Ok(ClipboardSource::Native),
            Self::Socket { path } => {
                let path = absolute_path(path).map_err(invalid)?;
                if path
                    .components()
                    .any(|component| component == Component::ParentDir)
                {
                    return Err(invalid("socket path must not contain '..' components"));
                }
                Ok(ClipboardSource::Socket { path })
            },
            Self::Ssh {
                host,
                reader,
                identity_file,
                known_hosts_file,
                port,
            } => {
                if host.is_empty()
                    || host.len() > 255
                    || host.starts_with('-')
                    || !host
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-@:[]".contains(&byte))
                {
                    return Err(invalid(
                        "SSH host must contain 1 to 255 ASCII letters, digits, or . _ - @ : [ ], and must not start with '-'",
                    ));
                }
                Ok(ClipboardSource::Ssh {
                    host,
                    reader,
                    identity_file: identity_file.map(ssh_path).transpose().map_err(invalid)?,
                    known_hosts_file: known_hosts_file
                        .map(ssh_path)
                        .transpose()
                        .map_err(invalid)?,
                    port,
                })
            },
        }
    }
}

fn ssh_path(value: String) -> Result<PathBuf, &'static str> {
    if value.contains(['%', '$']) {
        return Err("SSH paths must not contain '%' or '$' expansion tokens");
    }
    absolute_path(value)
}

fn absolute_path(value: String) -> Result<PathBuf, &'static str> {
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || !Path::new(&value).is_absolute()
    {
        return Err(
            "paths must be absolute and contain 1 to 4096 bytes without control characters",
        );
    }
    Ok(value.into())
}
