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

/// 크기가 제한된 `credentials.yaml`을 private exact-revision CAS로 관리하는 local repository입니다.
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

    /// file 또는 parent directory를 만들지 않고 capture합니다.
    pub fn capture(&self) -> Result<CredentialSnapshot, LocalCredentialStoreError> {
        storage::read_snapshot(&self.path).map(StoredCredentialSnapshot::public)
    }

    /// credential lock 아래에서 다시 읽어 candidate secret을 보관하지 않은 채 정확한 add 또는
    /// replace를 준비합니다.
    pub fn prepare_set(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedCredentialMutation, LocalCredentialStoreError> {
        self.prepare(provider, account, true)?
            .ok_or(LocalCredentialStoreError::InvalidMutation)
    }

    /// credential lock 아래에서 다시 읽어 정확한 removal을 준비합니다. 정확한 pair가 이미
    /// 없으면 `None`을 반환합니다.
    pub fn prepare_remove(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<Option<PreparedCredentialMutation>, LocalCredentialStoreError> {
        self.prepare(provider, account, false)
    }

    /// 기존 Provider-and-Account API credential의 optional account-session field에 대한 add 또는
    /// replacement를 준비합니다.
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

    /// 정확히 준비된 pair action을 commit합니다. Add와 replace에는 아직 메모리에 있는 candidate가
    /// 필요하고, remove에는 관련 없는 secret을 실수로 저장하지 않도록 candidate를 거부합니다.
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

    /// 같은 account record의 model API credential을 바꾸지 않고 준비된 account-session add 또는
    /// replacement 하나를 commit합니다.
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
