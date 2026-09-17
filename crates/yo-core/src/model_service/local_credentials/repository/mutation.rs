use std::fmt::{Debug, Formatter, Result as FmtResult};

use super::{
    super::super::LocalCredentialStoreError,
    model::{CredentialRevision, CredentialSnapshot},
};
use crate::model_service::{AccountId, ApiCredential, ProviderId};

/// 정확한 Provider-and-Account 쌍에 결합된 준비 mutation action입니다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialMutationAction {
    Add,
    Replace,
    Remove,
}

/// 좌표와 private revision을 담지만 secret byte는 보관하지 않는 준비 mutation입니다.
#[derive(Clone, Eq, PartialEq)]
pub struct PreparedCredentialMutation {
    expected_revision: CredentialRevision,
    planned_revision: CredentialRevision,
    provider: ProviderId,
    account: AccountId,
    action: CredentialMutationAction,
}

/// 기존 정확한 account의 optional account-session field를 위한 준비 mutation입니다.
///
/// candidate secret은 commit 시점까지 호출자에게 남아 있으며 이 타입에는 보관되지 않습니다.
#[derive(Clone, Eq, PartialEq)]
pub struct PreparedAccountSessionMutation {
    expected_revision: CredentialRevision,
    planned_revision: CredentialRevision,
    provider: ProviderId,
    account: AccountId,
    action: CredentialMutationAction,
}

impl PreparedCredentialMutation {
    pub(super) fn new(
        expected_revision: CredentialRevision,
        planned_revision: CredentialRevision,
        provider: ProviderId,
        account: AccountId,
        action: CredentialMutationAction,
    ) -> Self {
        Self {
            expected_revision,
            planned_revision,
            provider,
            account,
            action,
        }
    }

    #[must_use]
    pub const fn expected_revision(&self) -> &CredentialRevision {
        &self.expected_revision
    }

    #[must_use]
    pub const fn planned_revision(&self) -> &CredentialRevision {
        &self.planned_revision
    }

    #[must_use]
    pub const fn action(&self) -> CredentialMutationAction {
        self.action
    }

    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    pub(crate) fn from_operation_journal(
        expected_revision: CredentialRevision,
        planned_revision: CredentialRevision,
        provider: ProviderId,
        account: AccountId,
        action: CredentialMutationAction,
    ) -> Option<Self> {
        if planned_revision.managed_token().is_none() || expected_revision == planned_revision {
            return None;
        }
        Some(Self {
            expected_revision,
            planned_revision,
            provider,
            account,
            action,
        })
    }
}

impl PreparedAccountSessionMutation {
    pub(super) fn new(
        expected_revision: CredentialRevision,
        planned_revision: CredentialRevision,
        provider: ProviderId,
        account: AccountId,
        action: CredentialMutationAction,
    ) -> Self {
        Self {
            expected_revision,
            planned_revision,
            provider,
            account,
            action,
        }
    }

    #[must_use]
    pub const fn planned_revision(&self) -> &CredentialRevision {
        &self.planned_revision
    }

    pub(super) const fn expected_revision(&self) -> &CredentialRevision {
        &self.expected_revision
    }

    pub(super) const fn action(&self) -> CredentialMutationAction {
        self.action
    }

    pub(super) const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub(super) const fn account(&self) -> &AccountId {
        &self.account
    }
}

impl Debug for PreparedCredentialMutation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("PreparedCredentialMutation")
            .field("expected_revision", &self.expected_revision)
            .field("planned_revision", &self.planned_revision)
            .field("provider", &self.provider)
            .field("account", &self.account)
            .field("action", &self.action)
            .finish()
    }
}

impl Debug for PreparedAccountSessionMutation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("PreparedAccountSessionMutation")
            .field("expected_revision", &self.expected_revision)
            .field("planned_revision", &self.planned_revision)
            .field("provider", &self.provider)
            .field("account", &self.account)
            .field("action", &self.action)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialCommit {
    Committed,
    AlreadyCommitted,
}

/// 저장소 구현과 무관한 정확한 Provider-and-Account credential mutation 경계입니다.
pub trait CredentialRepository {
    fn capture(&self) -> Result<CredentialSnapshot, LocalCredentialStoreError>;

    fn prepare_set(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<PreparedCredentialMutation, LocalCredentialStoreError>;

    fn prepare_remove(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Result<Option<PreparedCredentialMutation>, LocalCredentialStoreError>;

    fn commit(
        &self,
        mutation: &PreparedCredentialMutation,
        candidate: Option<&ApiCredential>,
    ) -> Result<CredentialCommit, LocalCredentialStoreError>;
}

impl CredentialMutationAction {
    pub(super) const fn matches_presence(self, present: bool) -> bool {
        match self {
            Self::Add => !present,
            Self::Replace | Self::Remove => present,
        }
    }
}
