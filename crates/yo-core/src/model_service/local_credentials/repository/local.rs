use std::path::{Path, PathBuf};

use super::{
    super::{LocalCredentialStoreError, storage, wire},
    model::CredentialSnapshot,
    mutation::{
        CredentialCommit, CredentialMutationAction, CredentialRepository,
        PreparedAccountSessionMutation, PreparedCredentialMutation,
    },
    validation::{StoredCredentialSnapshot, validate_candidate},
};
use crate::model_service::{AccountId, ApiCredential, ProviderId};

/// Local bounded `credentials.yaml` repository with private exact-revision CAS.
#[derive(Clone, Debug)]
pub struct LocalCredentialRepository {
    path: PathBuf,
}

impl LocalCredentialRepository {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Captures without creating the file or its parent directory.
    pub fn capture(&self) -> Result<CredentialSnapshot, LocalCredentialStoreError> {
        storage::read_snapshot(&self.path).map(StoredCredentialSnapshot::public)
    }

    /// Re-reads under the credential lock and prepares an exact add or replace without retaining
    /// the candidate secret.
    pub fn prepare_set(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedCredentialMutation, LocalCredentialStoreError> {
        self.prepare(provider, account, true)?
            .ok_or(LocalCredentialStoreError::InvalidMutation)
    }

    /// Re-reads under the credential lock and prepares an exact removal, or returns `None` when
    /// the exact pair is already absent.
    pub fn prepare_remove(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<Option<PreparedCredentialMutation>, LocalCredentialStoreError> {
        self.prepare(provider, account, false)
    }

    /// Prepares an add or replacement of the optional account-session field for an existing
    /// Provider-and-Account API credential.
    pub fn prepare_set_account_session(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedAccountSessionMutation, LocalCredentialStoreError> {
        let (_parent, lock) = storage::lock_repository(&self.path)?;
        let snapshot = storage::read_snapshot(&self.path)?.public();
        let mutation = snapshot.prepare_set_account_session(provider, account)?;
        drop(lock);
        Ok(mutation)
    }

    fn prepare(
        &self,
        provider: &ProviderId,
        account: &AccountId,
        set: bool,
    ) -> Result<Option<PreparedCredentialMutation>, LocalCredentialStoreError> {
        let (_parent, lock) = storage::lock_repository(&self.path)?;
        let snapshot = storage::read_snapshot(&self.path)?;
        let present = snapshot.resolve(provider, account).is_some();
        let action = match (set, present) {
            (true, false) => CredentialMutationAction::Add,
            (true, true) => CredentialMutationAction::Replace,
            (false, true) => CredentialMutationAction::Remove,
            (false, false) => return Ok(None),
        };
        let mutation = PreparedCredentialMutation::new(
            snapshot.revision().clone(),
            super::model::new_revision()?,
            provider.clone(),
            account.clone(),
            action,
        );
        drop(lock);
        Ok(Some(mutation))
    }

    /// Commits the exact prepared pair action. Add and replace require the still in-memory
    /// candidate; remove rejects one so a caller cannot accidentally persist an unrelated secret.
    pub fn commit(
        &self,
        mutation: &PreparedCredentialMutation,
        candidate: Option<&ApiCredential>,
    ) -> Result<CredentialCommit, LocalCredentialStoreError> {
        validate_candidate(mutation.action(), candidate)?;
        let (parent, lock) = storage::lock_repository(&self.path)?;
        let mut current = storage::read_snapshot(&self.path)?;

        if current.revision() == mutation.planned_revision() {
            let applied = match mutation.action() {
                CredentialMutationAction::Add | CredentialMutationAction::Replace => {
                    current.resolve(mutation.provider(), mutation.account()) == candidate
                },
                CredentialMutationAction::Remove => current
                    .resolve(mutation.provider(), mutation.account())
                    .is_none(),
            };
            return if applied {
                Ok(CredentialCommit::AlreadyCommitted)
            } else {
                Err(LocalCredentialStoreError::Conflict(self.path.clone()))
            };
        }
        if current.revision() != mutation.expected_revision()
            || !mutation.action().matches_presence(
                current
                    .resolve(mutation.provider(), mutation.account())
                    .is_some(),
            )
        {
            return Err(LocalCredentialStoreError::Conflict(self.path.clone()));
        }

        current.apply(mutation, candidate);
        let planned = mutation
            .planned_revision()
            .managed_token()
            .expect("new credential revisions are always managed tokens");
        let encoded = wire::encode(planned, current.entries())?;
        storage::publish(
            &self.path,
            &parent,
            mutation.expected_revision().is_absent(),
            &encoded,
        )?;
        drop(lock);
        Ok(CredentialCommit::Committed)
    }

    /// Commits one prepared account-session add or replacement without changing the model API
    /// credential in the same account record.
    pub fn commit_account_session(
        &self,
        mutation: &PreparedAccountSessionMutation,
        candidate: &ApiCredential,
    ) -> Result<CredentialCommit, LocalCredentialStoreError> {
        let (parent, lock) = storage::lock_repository(&self.path)?;
        let mut current = storage::read_snapshot(&self.path)?;

        if current.revision() == mutation.planned_revision() {
            return if current.resolve_account_session(mutation.provider(), mutation.account())
                == Some(candidate)
            {
                Ok(CredentialCommit::AlreadyCommitted)
            } else {
                Err(LocalCredentialStoreError::Conflict(self.path.clone()))
            };
        }
        if current.revision() != mutation.expected_revision()
            || current
                .resolve(mutation.provider(), mutation.account())
                .is_none()
            || !mutation.action().matches_presence(
                current
                    .resolve_account_session(mutation.provider(), mutation.account())
                    .is_some(),
            )
        {
            return Err(LocalCredentialStoreError::Conflict(self.path.clone()));
        }

        current.apply_account_session(mutation, candidate);
        let planned = mutation
            .planned_revision()
            .managed_token()
            .expect("new credential revisions are always managed tokens");
        let encoded = wire::encode(planned, current.entries())?;
        storage::publish(
            &self.path,
            &parent,
            mutation.expected_revision().is_absent(),
            &encoded,
        )?;
        drop(lock);
        Ok(CredentialCommit::Committed)
    }
}

impl CredentialRepository for LocalCredentialRepository {
    fn capture(&self) -> Result<CredentialSnapshot, LocalCredentialStoreError> {
        Self::capture(self)
    }

    fn prepare_set(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedCredentialMutation, LocalCredentialStoreError> {
        Self::prepare_set(self, provider, account)
    }

    fn prepare_remove(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<Option<PreparedCredentialMutation>, LocalCredentialStoreError> {
        Self::prepare_remove(self, provider, account)
    }

    fn commit(
        &self,
        mutation: &PreparedCredentialMutation,
        candidate: Option<&ApiCredential>,
    ) -> Result<CredentialCommit, LocalCredentialStoreError> {
        Self::commit(self, mutation, candidate)
    }
}
