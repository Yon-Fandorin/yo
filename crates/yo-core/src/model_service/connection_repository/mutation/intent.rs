use std::path::{Path, PathBuf};

use super::super::{
    MAX_CONNECTION_BYTES,
    error::ConnectionRepositoryError,
    model::{
        ConnectionAccount, ConnectionCatalogSeed, ConnectionRevision, ConnectionSnapshot,
        StoredModelBinding,
    },
    wire::decode as decode_snapshot,
};
use crate::{AccountId, CompleteModelBinding, ProviderId, StartupTarget};

/// 캡처한 revision에서 준비한 불변의 정확한 public mutation입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedConnectionMutation {
    pub(in super::super) expected_revision: ConnectionRevision,
    pub(in super::super) planned_revision: ConnectionRevision,
    pub(in super::super) planned_bytes: Vec<u8>,
    pub(super) preference: Option<StartupTarget>,
    pub(super) direct_connect: Option<DirectConnectIntent>,
    pub(super) group_replacement: Option<GroupReplacementIntent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DirectConnectIntent {
    pub(super) account: ConnectionAccount,
    pub(super) binding: StoredModelBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct GroupReplacementIntent {
    pub(super) account: ConnectionAccount,
    pub(super) bindings: Vec<StoredModelBinding>,
    pub(super) catalog_seed: Option<ConnectionCatalogSeed>,
}

impl PreparedConnectionMutation {
    #[must_use]
    pub const fn expected_revision(&self) -> &ConnectionRevision {
        &self.expected_revision
    }

    #[must_use]
    pub const fn planned_revision(&self) -> &ConnectionRevision {
        &self.planned_revision
    }

    #[must_use]
    pub const fn preference(&self) -> Option<&StartupTarget> {
        self.preference.as_ref()
    }

    pub(crate) fn planned_bytes(&self) -> &[u8] {
        &self.planned_bytes
    }

    pub(crate) fn into_journal_mutation(mut self) -> Self {
        self.direct_connect = None;
        self.group_replacement = None;
        self
    }

    pub(crate) fn defines_model_connect(
        &self,
        provider: &ProviderId,
        account: &AccountId,
        complete_bindings: &[CompleteModelBinding],
    ) -> bool {
        let Some(intent) = self.direct_connect.as_ref() else {
            return false;
        };
        if intent.account.provider_id() != provider
            || intent.account.account_id() != account
            || !complete_bindings.contains(intent.binding.complete())
        {
            return false;
        }
        let Ok(decoded) = decode_snapshot(Path::new(""), &self.planned_bytes) else {
            return false;
        };
        let stored = decoded
            .bindings
            .iter()
            .filter(|binding| {
                let coordinate = binding.complete().binding();
                coordinate.provider_id() == provider && coordinate.account_id() == account
            })
            .map(StoredModelBinding::complete)
            .collect::<Vec<_>>();
        let same_bindings = stored.len() == complete_bindings.len()
            && stored
                .iter()
                .all(|binding| complete_bindings.contains(binding));
        let pair_exists = decoded
            .accounts
            .iter()
            .find(|stored| stored.provider_id() == provider && stored.account_id() == account)
            == Some(&intent.account);
        let exact_target = decoded
            .bindings
            .iter()
            .find(|stored| stored.selection() == intent.binding.selection())
            == Some(&intent.binding);
        pair_exists && exact_target && same_bindings && !complete_bindings.is_empty()
    }

    pub(crate) fn defines_group_replacement(
        &self,
        provider: &ProviderId,
        account: &AccountId,
        complete_bindings: &[CompleteModelBinding],
    ) -> bool {
        let Some(intent) = self.group_replacement.as_ref() else {
            return false;
        };
        let intended_complete = intent
            .bindings
            .iter()
            .map(StoredModelBinding::complete)
            .collect::<Vec<_>>();
        if intent.account.provider_id() != provider
            || intent.account.account_id() != account
            || intended_complete.len() != complete_bindings.len()
            || !intended_complete
                .iter()
                .all(|binding| complete_bindings.contains(binding))
        {
            return false;
        }
        let Ok(decoded) = decode_snapshot(Path::new(""), &self.planned_bytes) else {
            return false;
        };
        let stored = decoded
            .bindings
            .iter()
            .filter(|binding| {
                let coordinate = binding.complete().binding();
                coordinate.provider_id() == provider && coordinate.account_id() == account
            })
            .map(StoredModelBinding::complete)
            .collect::<Vec<_>>();
        let same_bindings = stored.len() == complete_bindings.len()
            && stored
                .iter()
                .all(|binding| complete_bindings.contains(binding));
        let exact_account = decoded
            .accounts
            .iter()
            .find(|stored| stored.provider_id() == provider && stored.account_id() == account)
            == Some(&intent.account);
        let exact_stored_bindings = stored.len() == intent.bindings.len()
            && intent.bindings.iter().all(|binding| {
                stored
                    .iter()
                    .any(|complete| *complete == binding.complete())
            });
        let decoded_seed = decoded
            .catalog_seeds
            .iter()
            .find(|seed| seed.provider() == provider && seed.account() == account);
        let exact_seed = decoded_seed == intent.catalog_seed.as_ref();
        let non_empty_group = !complete_bindings.is_empty() || intent.catalog_seed.is_some();
        exact_account && same_bindings && exact_stored_bindings && exact_seed && non_empty_group
    }

    pub(crate) fn from_operation_journal(
        expected_revision: ConnectionRevision,
        planned_revision: ConnectionRevision,
        planned_bytes: Vec<u8>,
    ) -> Result<Self, ConnectionRepositoryError> {
        if planned_revision.is_absent()
            || expected_revision == planned_revision
            || planned_bytes.len() as u64 > MAX_CONNECTION_BYTES
        {
            return Err(ConnectionRepositoryError::InvalidContents(PathBuf::new()));
        }
        let decoded = decode_snapshot(Path::new(""), &planned_bytes)?;
        if decoded.revision != planned_revision {
            return Err(ConnectionRepositoryError::InvalidContents(PathBuf::new()));
        }
        Ok(Self {
            expected_revision,
            planned_revision,
            planned_bytes,
            preference: decoded.preference,
            direct_connect: None,
            group_replacement: None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionCommit {
    Committed,
    AlreadyCommitted,
}

/// connection orchestration이 사용하는 storage 중립 preference publication 경계입니다.
pub trait ConnectionRepository {
    type OperationGuard;

    fn acquire_operation(&self) -> Result<Self::OperationGuard, ConnectionRepositoryError>;
    fn recover_pending_operation(&self) -> Result<(), ConnectionRepositoryError>;
    fn capture(&self) -> Result<ConnectionSnapshot, ConnectionRepositoryError>;
    fn commit(
        &self,
        mutation: &PreparedConnectionMutation,
    ) -> Result<ConnectionCommit, ConnectionRepositoryError>;
}
