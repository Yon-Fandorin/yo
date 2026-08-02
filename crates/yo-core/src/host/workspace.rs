use std::{
    fmt, io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

/// Opaque canonical workspace path produced by the Host that owns it.
///
/// The bytes remain host-owned so another Host never applies its local path
/// rules to a remote workspace.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HostWorkspacePath(Vec<u8>);

impl HostWorkspacePath {
    /// Resolves one local macOS or Linux workspace to its stable absolute path.
    pub fn normalize_local(path: impl AsRef<Path>) -> Result<Self, HostWorkspacePathError> {
        let path = path.as_ref();
        let canonical = std::fs::canonicalize(path).map_err(|source| HostWorkspacePathError {
            path: path.to_owned(),
            source,
        })?;
        Ok(Self(canonical.as_os_str().as_bytes().to_vec()))
    }

    pub fn as_unix_bytes(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn from_unix_bytes(bytes: Vec<u8>) -> Result<Self, &'static str> {
        if bytes.first() != Some(&b'/') {
            return Err("a host-normalized Unix workspace path must be absolute");
        }
        if bytes.contains(&0) {
            return Err("a host-normalized Unix workspace path cannot contain NUL");
        }
        if bytes.len() > 1 && bytes.last() == Some(&b'/') {
            return Err("a host-normalized Unix workspace path cannot end with a separator");
        }
        if bytes.len() > 1 {
            for component in bytes[1..].split(|byte| *byte == b'/') {
                if component.is_empty() {
                    return Err(
                        "a host-normalized Unix workspace path cannot contain empty components",
                    );
                }
                if component == b"." || component == b".." {
                    return Err(
                        "a host-normalized Unix workspace path cannot contain dot components",
                    );
                }
            }
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for HostWorkspacePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut remaining = self.as_unix_bytes();
        while !remaining.is_empty() {
            match std::str::from_utf8(remaining) {
                Ok(valid) => {
                    write_escaped(valid, formatter)?;
                    break;
                },
                Err(error) => {
                    let (valid, invalid) = remaining.split_at(error.valid_up_to());
                    write_escaped(
                        std::str::from_utf8(valid)
                            .expect("the UTF-8 validator reported this prefix"),
                        formatter,
                    )?;
                    let invalid_length = error.error_len().unwrap_or(1);
                    for byte in &invalid[..invalid_length] {
                        write!(formatter, "\\x{byte:02X}")?;
                    }
                    remaining = &invalid[invalid_length..];
                },
            }
        }
        Ok(())
    }
}

fn write_escaped(value: &str, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    for character in value.chars() {
        match character {
            '\\' => formatter.write_str("\\\\")?,
            character if character.is_control() => {
                for escaped in character.escape_default() {
                    write!(formatter, "{escaped}")?;
                }
            },
            character => write!(formatter, "{character}")?,
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct HostWorkspacePathError {
    path: PathBuf,
    source: io::Error,
}

impl fmt::Display for HostWorkspacePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to normalize local workspace path at {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for HostWorkspacePathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}
