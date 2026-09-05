use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use yo_core::ApiCredential;

use super::ExternalConnectInput;
use crate::{AppError, connection::presentation::ConfirmationView};

const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;
const MAX_FILE_BYTES: usize = MAX_CREDENTIAL_BYTES + 2;

#[cfg(target_vendor = "apple")]
const FILE_TYPE_MASK: u32 = libc::S_IFMT as u32;
#[cfg(not(target_vendor = "apple"))]
const FILE_TYPE_MASK: u32 = libc::S_IFMT;
#[cfg(target_vendor = "apple")]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG as u32;
#[cfg(not(target_vendor = "apple"))]
const REGULAR_FILE_MODE: u32 = libc::S_IFREG;

pub(crate) struct AuthorizedCredentialFileInput {
    path: PathBuf,
}

impl AuthorizedCredentialFileInput {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl ExternalConnectInput for AuthorizedCredentialFileInput {
    fn confirm(&mut self, _: &dyn ConfirmationView) -> Result<bool, AppError> {
        Ok(true)
    }

    fn read_credential(&mut self, _: &str) -> Result<ApiCredential, AppError> {
        read_credential_file(&self.path)
    }
}

fn read_credential_file(path: &Path) -> Result<ApiCredential, AppError> {
    read_credential_file_with(path, rustix::process::geteuid().as_raw(), || Ok(()))
}

fn read_credential_file_with(
    path: &Path,
    expected_user: u32,
    after_read: impl FnOnce() -> Result<(), AppError>,
) -> Result<ApiCredential, AppError> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| AppError::single("opening the credential file", error))?;
    let before = CredentialFileMetadata::capture(&file)?;
    before.validate(path, expected_user)?;

    let mut bytes = Vec::with_capacity(before.len.min(MAX_FILE_BYTES as u64) as usize);
    Read::by_ref(&mut file)
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::single("reading the credential file", error))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AppError::message(
            "credential file exceeds the 16,386-byte limit",
        ));
    }
    after_read()?;
    let after = CredentialFileMetadata::capture(&file)?;
    if before != after || bytes.len() as u64 != after.len {
        return Err(AppError::message(
            "credential file changed while it was being read; retry with a stable file",
        ));
    }

    if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
    } else if bytes.ends_with(b"\n") {
        bytes.pop();
    }
    if bytes.len() > MAX_CREDENTIAL_BYTES {
        return Err(AppError::message(
            "credential value exceeds the 16,384-byte limit",
        ));
    }
    let value = String::from_utf8(bytes)
        .map_err(|_| AppError::message("credential file must contain valid UTF-8"))?;
    ApiCredential::new(value).map_err(|error| AppError::single("reading the API key", error))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CredentialFileMetadata {
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

impl CredentialFileMetadata {
    fn capture(file: &File) -> Result<Self, AppError> {
        let metadata = file
            .metadata()
            .map_err(|error| AppError::single("inspecting the credential file", error))?;
        Ok(Self {
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

    fn validate(&self, path: &Path, expected_user: u32) -> Result<(), AppError> {
        if self.mode & FILE_TYPE_MASK != REGULAR_FILE_MODE {
            return Err(AppError::message(format!(
                "credential file must be a regular file: {}",
                path.display()
            )));
        }
        if self.user != expected_user {
            return Err(AppError::message(format!(
                "credential file must be owned by the current user: {}",
                path.display()
            )));
        }
        let permissions = self.mode & 0o7777;
        if !matches!(permissions, 0o400 | 0o600) {
            return Err(AppError::message(format!(
                "credential file permissions must be exactly 0400 or 0600: {}",
                path.display()
            )));
        }
        if self.len > MAX_FILE_BYTES as u64 {
            return Err(AppError::message(
                "credential file exceeds the 16,386-byte limit",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
