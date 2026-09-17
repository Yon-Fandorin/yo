use std::collections::{HashMap, HashSet, hash_map::Entry};

use super::error::ConnectionRepositoryError;
use crate::{
    AccountId, ModelCatalog, ModelCatalogEntry, ModelSelection, ModelServiceError, ProviderId,
    StartupTarget,
};

pub(super) mod account;
pub(super) mod binding;
pub(super) mod catalog;
pub(super) mod failure;
pub(super) mod revision;

pub use account::ConnectionAccount;
pub use binding::StoredModelBinding;
pub(super) use catalog::CatalogSource;
pub use catalog::ConnectionCatalogSeed;
pub use failure::{ModelLastFailure, ModelRequestFailureKind};
pub use revision::ConnectionRevision;

/// 크기가 제한된 불변 공개 저장소 스냅샷입니다.
#[derive(Clone, Debug)]
pub struct ConnectionSnapshot {
    pub(in super::super) revision: ConnectionRevision,
    pub(in super::super) preference: Option<StartupTarget>,
    pub(in super::super) accounts: Vec<ConnectionAccount>,
    pub(in super::super) bindings: Vec<StoredModelBinding>,
    pub(in super::super) catalog_seeds: Vec<ConnectionCatalogSeed>,
    pub(in super::super) encoded: Vec<u8>,
}

impl ConnectionSnapshot {
    #[must_use]
    pub const fn revision(&self) -> &ConnectionRevision {
        &self.revision
    }

    #[must_use]
    pub const fn preference(&self) -> Option<&StartupTarget> {
        self.preference.as_ref()
    }

    #[must_use]
    pub fn accounts(&self) -> &[ConnectionAccount] {
        &self.accounts
    }

    #[must_use]
    pub fn models(&self) -> &[StoredModelBinding] {
        &self.bindings
    }

    #[must_use]
    pub fn catalog_seeds(&self) -> &[ConnectionCatalogSeed] {
        &self.catalog_seeds
    }

    /// 서비스 해석 없이 정확히 일치하는 Provider-and-Account 영속 seed만 반환합니다.
    pub fn catalog_seed(
        &self,
        provider: &ProviderId,
        account: &AccountId,
    ) -> Option<&ConnectionCatalogSeed> {
        self.catalog_seeds
            .iter()
            .find(|seed| seed.provider() == provider && seed.account() == account)
    }

    pub fn model_catalog(&self) -> Result<ModelCatalog, ConnectionRepositoryError> {
        let entries = self
            .bindings
            .iter()
            .map(|binding| {
                let complete = binding.complete().binding();
                let account = self.accounts.iter().find(|account| {
                    account.provider_id() == complete.provider_id()
                        && account.account_id() == complete.account_id()
                });
                let account = account.ok_or(ConnectionRepositoryError::InvalidMutation)?;
                ModelCatalogEntry::from_stored(
                    binding.complete().clone(),
                    account.provider_display_name().map(str::to_owned),
                    account.account_display_name().map(str::to_owned),
                    binding.model_display_name().map(str::to_owned),
                    binding.last_failure().cloned(),
                    binding.is_enabled(),
                )
                .map_err(|_| ConnectionRepositoryError::InvalidMutation)
            })
            .collect::<Result<Vec<_>, _>>()?;
        ModelCatalog::new(entries).map_err(|_| ConnectionRepositoryError::InvalidMutation)
    }
}

/// mutation 계획과 local publication이 공유하는 디코딩된 연결 상태입니다.
#[derive(Clone, Debug)]
pub(in super::super) struct DecodedSnapshot {
    pub(in super::super) revision: ConnectionRevision,
    pub(in super::super) preference: Option<StartupTarget>,
    pub(in super::super) accounts: Vec<ConnectionAccount>,
    pub(in super::super) bindings: Vec<StoredModelBinding>,
    pub(in super::super) catalog_seeds: Vec<ConnectionCatalogSeed>,
}
pub(in super::super) fn validate_state(
    accounts: &[ConnectionAccount],
    bindings: &[StoredModelBinding],
) -> Result<(), ModelServiceError> {
    let mut account_coordinates = HashSet::new();
    let mut provider_display_names = HashMap::new();
    for account in accounts {
        let coordinate = (account.provider_id().clone(), account.account_id().clone());
        if !account_coordinates.insert(coordinate.clone()) {
            return Err(ModelServiceError::new(format!(
                "duplicate stored account for Provider {} and Account {}",
                account.provider_id(),
                account.account_id()
            )));
        }
        require_consistent_provider_display(
            &mut provider_display_names,
            account.provider_id().clone(),
            account.provider_display_name(),
        )?;
    }

    let mut binding_coordinates = HashSet::new();
    for binding in bindings {
        let complete = binding.complete().binding();
        let account_coordinate = (
            complete.provider_id().clone(),
            complete.account_id().clone(),
        );
        if !account_coordinates.contains(&account_coordinate) {
            return Err(ModelServiceError::new(format!(
                "stored model for Provider {}, Account {}, Model {} has no stored account",
                complete.provider_id(),
                complete.account_id(),
                complete.model_id()
            )));
        }
        let coordinate = (
            complete.provider_id().clone(),
            complete.account_id().clone(),
            complete.model_id().clone(),
        );
        if !binding_coordinates.insert(coordinate) {
            return Err(ModelServiceError::new(format!(
                "duplicate stored model for Provider {}, Account {}, Model {}",
                complete.provider_id(),
                complete.account_id(),
                complete.model_id()
            )));
        }
    }
    Ok(())
}

pub(in super::super) fn account_matches_binding(
    account: &ConnectionAccount,
    binding: &StoredModelBinding,
) -> bool {
    let complete = binding.complete().binding();
    account.provider_id() == complete.provider_id() && account.account_id() == complete.account_id()
}

pub(in super::super) fn binding_matches_selection(
    binding: &StoredModelBinding,
    selection: &ModelSelection,
) -> bool {
    let complete = binding.complete().binding();
    complete.provider_id() == selection.provider()
        && complete.account_id() == selection.account()
        && complete.model_id() == selection.model()
}

pub(super) fn reject_new_host_provider(provider_id: &ProviderId) -> Result<(), ModelServiceError> {
    if provider_id.as_str() == "host" {
        return Err(ModelServiceError::new(
            "new stored connections cannot use the reserved ProviderId host",
        ));
    }
    Ok(())
}

fn require_consistent_provider_display(
    names: &mut HashMap<ProviderId, Option<String>>,
    provider_id: ProviderId,
    value: Option<&str>,
) -> Result<(), ModelServiceError> {
    let value = value.map(str::to_owned);
    match names.entry(provider_id.clone()) {
        Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        },
        Entry::Occupied(entry) if entry.get() == &value => Ok(()),
        Entry::Occupied(_) => Err(ModelServiceError::new(format!(
            "inconsistent stored display name for Provider {provider_id}"
        ))),
    }
}
