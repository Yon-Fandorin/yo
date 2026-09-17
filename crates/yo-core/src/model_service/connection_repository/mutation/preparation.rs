use super::{
    super::{
        MAX_CONNECTION_BYTES,
        error::ConnectionRepositoryError,
        model::{
            ConnectionAccount, ConnectionCatalogSeed, ConnectionRevision, ConnectionSnapshot,
            ModelLastFailure, StoredModelBinding, account_matches_binding,
            binding_matches_selection, catalog::validate_catalog_seeds, validate_state,
        },
        wire::encode as encode_snapshot,
    },
    intent::{DirectConnectIntent, GroupReplacementIntent, PreparedConnectionMutation},
};
use crate::{CompleteModelBinding, ModelCatalog, ModelSelection, StartupTarget};

fn new_revision() -> Result<ConnectionRevision, ConnectionRepositoryError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| ConnectionRepositoryError::Randomness(error.to_string()))?;
    let mut token = String::with_capacity(36);
    token.push_str("rev-");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(token, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    Ok(ConnectionRevision::Token(token))
}

type StoredSnapshotState = (
    Option<StartupTarget>,
    Vec<ConnectionAccount>,
    Vec<StoredModelBinding>,
    Vec<ConnectionCatalogSeed>,
);

impl ConnectionSnapshot {
    /// stored upsert 하나를 적용한 뒤의 정확한 예정 catalog를 구성합니다.
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

    /// stored removal 하나를 적용한 뒤의 정확한 예정 catalog를 구성합니다.
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

    /// 저장소를 변경하지 않고 정확한 이전 또는 새 bytes를 준비합니다.
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

    /// 관련 없는 공개 상태를 보존하면서 stored model 하나를 추가하거나 교체합니다.
    pub fn prepare_model_upsert(
        &self,
        account: ConnectionAccount,
        binding: StoredModelBinding,
    ) -> Result<Option<PreparedConnectionMutation>, ConnectionRepositoryError> {
        let (preference, accounts, bindings, catalog_seeds) =
            self.model_upsert_state(account, binding)?;
        self.prepare_snapshot(preference, accounts, bindings, catalog_seeds)
    }

    /// 공개 binding이 바뀌지 않아도 외부 connect epoch를 준비합니다.
    ///
    /// Credential 교체 recovery에는 정확한 예정 public revision이 필요합니다. key를 교체해도
    /// 의미상 동일한 상태를 보존하면서 새 public revision을 부여합니다.
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

    /// 하나의 complete Provider-and-Account 정의를 하나의 public revision으로 교체합니다.
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

    /// 정확히 일치하는 현재 complete binding에 대한 warning-only 관찰 갱신을 준비합니다.
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

    /// complete identity를 바꾸지 않고 정확히 일치하는 stored binding 하나를 활성화하거나
    /// 비활성화합니다.
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

    /// stored model 하나와 사용되지 않는 account, 정확히 일치하는 preference를 제거합니다.
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
