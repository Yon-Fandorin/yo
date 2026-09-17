use super::{
    super::super::LocalCredentialStoreError,
    model::{CredentialRevision, CredentialSnapshot},
};
use crate::model_service::{AccountId, ApiCredential, ProviderId};

/// Closed exact-pair action bound into a prepared credential mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialMutationAction {
    Add,
    Replace,
    Remove,
}

/// One prepared mutation. It contains coordinates and private revisions but never secret bytes.
#[derive(Clone, Eq, PartialEq)]
pub struct PreparedCredentialMutation {
    expected_revision: CredentialRevision,
    planned_revision: CredentialRevision,
    provider: ProviderId,
    account: AccountId,
    action: CredentialMutationAction,
}

/// One prepared mutation of the optional account-session field for an existing exact account.
///
/// The candidate secret remains in the caller until commit and is never retained here.
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

impl std::fmt::Debug for PreparedCredentialMutation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

impl std::fmt::Debug for PreparedAccountSessionMutation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// Storage-neutral exact-pair credential mutation boundary.
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
