use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use super::{
    ConnectionAccount, ConnectionCatalogSeed, ConnectionRepositoryError, ConnectionRevision,
    ConnectionSnapshot, MAX_CONNECTION_BYTES, ModelLastFailure, StoredModelBinding,
    account_matches_binding, binding_matches_selection, decode_snapshot, encode_snapshot,
    new_revision, validate_state,
};
use crate::{
    AccountId, CompleteModelBinding, ModelCatalog, ModelSelection, ProviderId, StartupTarget,
};

type StoredSnapshotState = (
    Option<StartupTarget>,
    Vec<ConnectionAccount>,
    Vec<StoredModelBinding>,
    Vec<ConnectionCatalogSeed>,
);

impl ConnectionSnapshot {
    /// Builds the exact prospective catalog after one stored upsert.
    pub fn catalog_after_model_upsert(
        &self,
        account: ConnectionAccount,
        binding: StoredModelBinding,
    ) -> Result<ModelCatalog, ConnectionRepositoryError> {
        let (preference, accounts, bindings, catalog_seeds) =
            self.model_upsert_state(account, binding)?;
        Self {
            revision: self.revision.clone(),
            preference,
            accounts,
            bindings,
            catalog_seeds,
            encoded: Vec::new(),
        }
        .model_catalog()
    }

    /// Builds the exact prospective catalog after one stored removal.
    pub fn catalog_after_model_remove(
        &self,
        selection: &ModelSelection,
    ) -> Result<ModelCatalog, ConnectionRepositoryError> {
        let (preference, accounts, bindings, catalog_seeds) = self.model_remove_state(selection)?;
        Self {
            revision: self.revision.clone(),
            preference,
            accounts,
            bindings,
            catalog_seeds,
            encoded: Vec::new(),
        }
        .model_catalog()
    }

    pub(crate) fn matches_planned(&self, mutation: &PreparedConnectionMutation) -> bool {
        self.revision == mutation.planned_revision && self.encoded == mutation.planned_bytes
    }

    /// Prepares exact old-or-new bytes without changing the repository.
    pub fn prepare_preference(
        &self,
        preference: Option<StartupTarget>,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        if self.preference == preference {
            return Ok(None);
        }
        self.prepare_snapshot(
            preference,
            self.accounts.clone(),
            self.bindings.clone(),
            self.catalog_seeds.clone(),
        )
    }

    /// Adds or replaces one stored model while preserving unrelated public state.
    pub fn prepare_model_upsert(
        &self,
        account: ConnectionAccount,
        binding: StoredModelBinding,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        let (preference, accounts, bindings, catalog_seeds) =
            self.model_upsert_state(account, binding)?;
        self.prepare_snapshot(preference, accounts, bindings, catalog_seeds)
    }

    /// Prepares an external connect epoch even when its public binding is unchanged.
    ///
    /// Credential replacement recovery needs an exact prospective public revision. Rotating a
    /// key therefore receives a new public revision while preserving semantically equal state.
    pub fn prepare_model_connect(
        &self,
        account: ConnectionAccount,
        binding: StoredModelBinding,
    ) -> Result<PreparedConnectionMutation, ConnectionRepositoryError> {
        let direct_connect = DirectConnectIntent {
            account: account.clone(),
            binding: binding.clone(),
        };
        let (preference, accounts, bindings, catalog_seeds) =
            self.model_upsert_state(account, binding)?;
        let mut mutation = self
            .prepare_snapshot_with_mode(preference, accounts, bindings, catalog_seeds, true)?
            .ok_or(ConnectionRepositoryError::InvalidMutation)?;
        mutation.direct_connect = Some(direct_connect);
        Ok(mutation)
    }

    fn model_upsert_state(
        &self,
        account: ConnectionAccount,
        mut binding: StoredModelBinding,
    ) -> Result<StoredSnapshotState, ConnectionRepositoryError> {
        if !account_matches_binding(&account, &binding) {
            return Err(ConnectionRepositoryError::CoordinateMismatch);
        }
        let mut accounts = self.accounts.clone();
        let account_position = accounts.iter().position(|current| {
            current.provider_id() == account.provider_id()
                && current.account_id() == account.account_id()
        });
        match account_position {
            Some(index) => accounts[index] = account,
            None => accounts.push(account),
        }

        let mut bindings = self.bindings.clone();
        let selection = binding.selection();
        let binding_position = bindings
            .iter()
            .position(|current| binding_matches_selection(current, &selection));
        let inserted = binding_position.is_none();
        match binding_position {
            Some(index) => {
                if bindings[index].complete() == binding.complete() {
                    binding = binding.with_enabled(bindings[index].is_enabled());
                }
                bindings[index] = binding;
            },
            None => bindings.push(binding),
        }
        validate_state(&accounts, &bindings)
            .map_err(|_| ConnectionRepositoryError::InvalidMutation)?;
        let preference = self
            .preference
            .clone()
            .or_else(|| inserted.then_some(StartupTarget::Model(selection)));
        Ok((preference, accounts, bindings, self.catalog_seeds.clone()))
    }

    /// Replaces one complete Provider-and-Account definition as one public revision.
    pub fn prepare_group_replace(
        &self,
        account: ConnectionAccount,
        mut replacement_bindings: Vec<StoredModelBinding>,
        replacement_seed: Option<ConnectionCatalogSeed>,
    ) -> Result<PreparedConnectionMutation, ConnectionRepositoryError> {
        if replacement_bindings
            .iter()
            .any(|binding| !account_matches_binding(&account, binding))
            || replacement_seed.as_ref().is_some_and(|seed| {
                seed.provider() != account.provider_id() || seed.account() != account.account_id()
            })
        {
            return Err(ConnectionRepositoryError::CoordinateMismatch);
        }
        if replacement_bindings.is_empty() && replacement_seed.is_none() {
            return Err(ConnectionRepositoryError::InvalidMutation);
        }

        let group_replacement = GroupReplacementIntent {
            account: account.clone(),
            bindings: replacement_bindings.clone(),
            catalog_seed: replacement_seed.clone(),
        };

        let provider = account.provider_id().clone();
        let account_id = account.account_id().clone();
        for replacement in &mut replacement_bindings {
            if let Some(retained) = self.bindings.iter().find(|current| {
                current.selection() == replacement.selection()
                    && current.complete() == replacement.complete()
            }) {
                *replacement = replacement
                    .clone()
                    .with_enabled(retained.is_enabled())
                    .with_last_failure(retained.last_failure().cloned());
            }
        }
        let mut accounts = self.accounts.clone();
        accounts.retain(|current| {
            current.provider_id() != &provider || current.account_id() != &account_id
        });
        accounts.push(account);
        let mut bindings = self.bindings.clone();
        bindings.retain(|current| {
            let binding = current.complete().binding();
            binding.provider_id() != &provider || binding.account_id() != &account_id
        });
        bindings.extend(replacement_bindings);
        let mut catalog_seeds = self.catalog_seeds.clone();
        catalog_seeds.retain(|seed| seed.provider() != &provider || seed.account() != &account_id);
        catalog_seeds.extend(replacement_seed);
        validate_state(&accounts, &bindings)
            .map_err(|_| ConnectionRepositoryError::InvalidMutation)?;
        validate_catalog_seeds(&accounts, &catalog_seeds)?;

        let preference = match self.preference.as_ref() {
            Some(StartupTarget::Model(selection))
                if selection.provider() == &provider && selection.account() == &account_id =>
            {
                bindings
                    .iter()
                    .any(|binding| binding.selection() == *selection)
                    .then(|| StartupTarget::Model(selection.clone()))
            },
            _ => self.preference.clone(),
        };
        let mut mutation = self
            .prepare_snapshot_with_mode(preference, accounts, bindings, catalog_seeds, true)?
            .ok_or(ConnectionRepositoryError::InvalidMutation)?;
        mutation.group_replacement = Some(group_replacement);
        Ok(mutation)
    }

    /// Prepares one warning-only observation update for an exact current complete binding.
    pub fn prepare_model_observation(
        &self,
        selection: &ModelSelection,
        expected_binding: &CompleteModelBinding,
        last_failure: Option<ModelLastFailure>,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        let Some(index) = self
            .bindings
            .iter()
            .position(|binding| binding_matches_selection(binding, selection))
        else {
            return Ok(None);
        };
        let current = &self.bindings[index];
        if current.complete() != expected_binding || current.last_failure() == last_failure.as_ref()
        {
            return Ok(None);
        }
        let mut bindings = self.bindings.clone();
        bindings[index] = current.clone().with_last_failure(last_failure);
        self.prepare_snapshot(
            self.preference.clone(),
            self.accounts.clone(),
            bindings,
            self.catalog_seeds.clone(),
        )
    }

    /// Enables or disables one exact stored binding without changing its complete identity.
    pub fn prepare_model_activation(
        &self,
        selection: &ModelSelection,
        enabled: bool,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        let Some(index) = self
            .bindings
            .iter()
            .position(|binding| binding_matches_selection(binding, selection))
        else {
            return Err(ConnectionRepositoryError::ModelNotFound {
                provider: selection.provider().to_string(),
                account: selection.account().to_string(),
                model: selection.model().to_string(),
            });
        };
        if self.bindings[index].is_enabled() == enabled {
            return Ok(None);
        }
        let mut bindings = self.bindings.clone();
        bindings[index] = bindings[index].clone().with_enabled(enabled);
        let disabled_target = StartupTarget::Model(selection.clone());
        let preference = if !enabled && self.preference.as_ref() == Some(&disabled_target) {
            None
        } else {
            self.preference.clone()
        };
        self.prepare_snapshot(
            preference,
            self.accounts.clone(),
            bindings,
            self.catalog_seeds.clone(),
        )
    }

    /// Removes one stored model, its unused account, and an exact matching preference.
    pub fn prepare_model_remove(
        &self,
        selection: &ModelSelection,
    ) -> Result<PreparedConnectionMutation, ConnectionRepositoryError> {
        let (preference, accounts, bindings, catalog_seeds) = self.model_remove_state(selection)?;
        self.prepare_snapshot(preference, accounts, bindings, catalog_seeds)?
            .ok_or(ConnectionRepositoryError::InvalidMutation)
    }

    fn model_remove_state(
        &self,
        selection: &ModelSelection,
    ) -> Result<StoredSnapshotState, ConnectionRepositoryError> {
        let mut bindings = self.bindings.clone();
        let Some(index) = bindings
            .iter()
            .position(|binding| binding_matches_selection(binding, selection))
        else {
            return Err(ConnectionRepositoryError::ModelNotFound {
                provider: selection.provider().to_string(),
                account: selection.account().to_string(),
                model: selection.model().to_string(),
            });
        };
        bindings.remove(index);
        let account_is_still_used = bindings.iter().any(|binding| {
            let complete = binding.complete().binding();
            complete.provider_id() == selection.provider()
                && complete.account_id() == selection.account()
        }) || self.catalog_seeds.iter().any(|seed| {
            seed.provider() == selection.provider() && seed.account() == selection.account()
        });
        let mut accounts = self.accounts.clone();
        if !account_is_still_used {
            accounts.retain(|account| {
                account.provider_id() != selection.provider()
                    || account.account_id() != selection.account()
            });
        }
        validate_state(&accounts, &bindings)
            .map_err(|_| ConnectionRepositoryError::InvalidMutation)?;
        let removed_target = StartupTarget::Model(selection.clone());
        let preference = if self.preference.as_ref() == Some(&removed_target) {
            None
        } else {
            self.preference.clone()
        };
        Ok((preference, accounts, bindings, self.catalog_seeds.clone()))
    }

    fn prepare_snapshot(
        &self,
        preference: Option<StartupTarget>,
        accounts: Vec<ConnectionAccount>,
        bindings: Vec<StoredModelBinding>,
        catalog_seeds: Vec<ConnectionCatalogSeed>,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        self.prepare_snapshot_with_mode(preference, accounts, bindings, catalog_seeds, false)
    }

    fn prepare_snapshot_with_mode(
        &self,
        preference: Option<StartupTarget>,
        accounts: Vec<ConnectionAccount>,
        bindings: Vec<StoredModelBinding>,
        catalog_seeds: Vec<ConnectionCatalogSeed>,
        force_new_revision: bool,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        if !force_new_revision
            && self.preference == preference
            && self.accounts == accounts
            && self.bindings == bindings
            && self.catalog_seeds == catalog_seeds
        {
            return Ok(None);
        }
        let planned_revision = new_revision()?;
        let planned_bytes = encode_snapshot(
            &planned_revision,
            preference.as_ref(),
            &accounts,
            &bindings,
            &catalog_seeds,
        )?;
        if planned_bytes.len() as u64 > MAX_CONNECTION_BYTES {
            return Err(ConnectionRepositoryError::PreparedTooLarge);
        }
        Ok(Some(PreparedConnectionMutation {
            expected_revision: self.revision.clone(),
            planned_revision,
            planned_bytes,
            preference,
            direct_connect: None,
            group_replacement: None,
        }))
    }
}

pub(in super::super) fn validate_catalog_seeds(
    accounts: &[ConnectionAccount],
    seeds: &[ConnectionCatalogSeed],
) -> Result<(), ConnectionRepositoryError> {
    let mut coordinates = HashSet::new();
    for seed in seeds {
        if !coordinates.insert((seed.provider().clone(), seed.account().clone()))
            || !accounts.iter().any(|account| {
                account.provider_id() == seed.provider() && account.account_id() == seed.account()
            })
        {
            return Err(ConnectionRepositoryError::InvalidMutation);
        }
    }
    Ok(())
}

/// One immutable exact public mutation prepared from a captured revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedConnectionMutation {
    pub(in super::super) expected_revision: ConnectionRevision,
    pub(in super::super) planned_revision: ConnectionRevision,
    pub(in super::super) planned_bytes: Vec<u8>,
    preference: Option<StartupTarget>,
    direct_connect: Option<DirectConnectIntent>,
    group_replacement: Option<GroupReplacementIntent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectConnectIntent {
    account: ConnectionAccount,
    binding: StoredModelBinding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GroupReplacementIntent {
    account: ConnectionAccount,
    bindings: Vec<StoredModelBinding>,
    catalog_seed: Option<ConnectionCatalogSeed>,
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

/// Storage-neutral preference publication boundary used by connection orchestration.
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
