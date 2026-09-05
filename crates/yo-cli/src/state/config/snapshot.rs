use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

use super::ConfigError;

pub(super) const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ConfigSnapshot {
    metadata: Option<ConfigMetadata>,
    bytes: Vec<u8>,
}

impl ConfigSnapshot {
    pub(super) fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            metadata: None,
            bytes,
        }
    }

    pub(super) fn absent() -> Self {
        Self::from_bytes(Vec::new())
    }

    pub(super) fn is_absent(&self) -> bool {
        self.metadata.is_none()
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfigMetadata {
    device: u64,
    inode: u64,
    mode: u32,
    user: u32,
    group: u32,
    len: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

pub(super) fn capture_snapshot(path: &Path) -> Result<ConfigSnapshot, ConfigError> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConfigSnapshot::absent());
        },
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.to_owned(),
                source,
            });
        },
    };
    let before = config_metadata(path, &file)?;
    if before.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
        return Err(ConfigError::UnsupportedFileType(path.to_owned()));
    }
    if before.len > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge {
            path: path.to_owned(),
            limit: MAX_CONFIG_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(before.len.min(MAX_CONFIG_BYTES)).unwrap_or(MAX_CONFIG_BYTES as usize),
    );
    Read::by_ref(&mut file)
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge {
            path: path.to_owned(),
            limit: MAX_CONFIG_BYTES,
        });
    }
    let after = config_metadata(path, &file)?;
    if before != after {
        return Err(ConfigError::Changed(path.to_owned()));
    }
    Ok(ConfigSnapshot {
        metadata: Some(before),
        bytes,
    })
}

fn config_metadata(path: &Path, file: &fs::File) -> Result<ConfigMetadata, ConfigError> {
    let metadata = file.metadata().map_err(|source| ConfigError::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(ConfigMetadata {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        user: metadata.uid(),
        group: metadata.gid(),
        len: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    })
}
