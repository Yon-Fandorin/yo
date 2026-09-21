//! 인터뷰 초안과 분리된 인증된 로컬 비밀 저장소입니다.

use std::{error::Error, fmt, io, result};

use rustix::io::Errno;

mod crypto;
mod filesystem;
mod model;
mod repository;

pub use model::{LiveAuthenticatedAccount, RetentionPolicy, SecretDestination, SecretMetadata};
pub use repository::SecretStore;

/// 비밀이나 파일 내용을 포함하지 않는 제한된 저장소 오류입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecretStoreError;

impl fmt::Display for SecretStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("Local secret storage is unavailable; retained files were not replaced.")
    }
}

impl Error for SecretStoreError {}

impl From<io::Error> for SecretStoreError {
    fn from(_: io::Error) -> Self {
        Self
    }
}

impl From<Errno> for SecretStoreError {
    fn from(_: Errno) -> Self {
        Self
    }
}

type Result<T> = result::Result<T, SecretStoreError>;

#[cfg(test)]
mod tests;
