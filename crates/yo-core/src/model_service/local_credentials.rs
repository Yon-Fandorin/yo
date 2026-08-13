use std::path::Path;

use super::CredentialStore;

mod error;
mod repository;
mod storage;
mod wire;

pub use error::LocalCredentialStoreError;
pub use repository::{
    CredentialCommit, CredentialMutationAction, CredentialRepository, CredentialRevision,
    CredentialSnapshot, LocalCredentialRepository, PreparedCredentialMutation,
};

/// Backward-compatible startup reader for `credentials.yaml`.
pub struct LocalCredentialStore;

impl LocalCredentialStore {
    pub fn open(path: impl AsRef<Path>) -> Result<CredentialStore, LocalCredentialStoreError> {
        storage::read_snapshot(path.as_ref()).map(|snapshot| snapshot.into_credentials())
    }
}

#[cfg(test)]
mod tests;
