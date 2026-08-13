use std::collections::{HashMap, HashSet, hash_map::Entry};

use super::super::{
    AccountId, CompleteModelBinding, ModelSelection, ModelServiceError, ProviderId,
    catalog::validate_display_name,
};

/// One durable managed account and its optional presentation metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedConnectionAccount {
    provider_id: ProviderId,
    account_id: AccountId,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
}

impl ManagedConnectionAccount {
    pub fn new(
        provider_id: ProviderId,
        account_id: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        reject_new_host_provider(&provider_id)?;
        Self::from_durable(
            provider_id,
            account_id,
            provider_display_name,
            account_display_name,
        )
    }

    pub(super) fn from_durable(
        provider_id: ProviderId,
        account_id: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        validate_display_name("Provider", provider_display_name.as_deref())?;
        validate_display_name("Account", account_display_name.as_deref())?;
        Ok(Self {
            provider_id,
            account_id,
            provider_display_name,
            account_display_name,
        })
    }

    #[must_use]
    pub const fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    #[must_use]
    pub const fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    #[must_use]
    pub fn provider_display_name(&self) -> Option<&str> {
        self.provider_display_name.as_deref()
    }

    #[must_use]
    pub fn account_display_name(&self) -> Option<&str> {
        self.account_display_name.as_deref()
    }
}

/// One durable managed model binding and its model presentation metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedConnectionBinding {
    complete: CompleteModelBinding,
    model_display_name: Option<String>,
}

impl ManagedConnectionBinding {
    pub fn new(
        complete: CompleteModelBinding,
        model_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        reject_new_host_provider(complete.binding().provider_id())?;
        Self::from_durable(complete, model_display_name)
    }

    pub(super) fn from_durable(
        complete: CompleteModelBinding,
        model_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        validate_display_name("Model", model_display_name.as_deref())?;
        Ok(Self {
            complete,
            model_display_name,
        })
    }

    #[must_use]
    pub const fn complete(&self) -> &CompleteModelBinding {
        &self.complete
    }

    #[must_use]
    pub fn model_display_name(&self) -> Option<&str> {
        self.model_display_name.as_deref()
    }

    #[must_use]
    pub fn selection(&self) -> ModelSelection {
        let binding = self.complete.binding();
        ModelSelection::new(
            binding.provider_id().clone(),
            binding.account_id().clone(),
            binding.model_id().clone(),
        )
    }
}

pub(super) fn validate_state(
    accounts: &[ManagedConnectionAccount],
    bindings: &[ManagedConnectionBinding],
) -> Result<(), ModelServiceError> {
    let mut account_coordinates = HashSet::new();
    let mut provider_display_names = HashMap::new();
    for account in accounts {
        let coordinate = (account.provider_id().clone(), account.account_id().clone());
        if !account_coordinates.insert(coordinate.clone()) {
            return Err(ModelServiceError::new(format!(
                "duplicate managed account for Provider {} and Account {}",
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
                "managed binding for Provider {}, Account {}, Model {} has no managed account",
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
                "duplicate managed binding for Provider {}, Account {}, Model {}",
                complete.provider_id(),
                complete.account_id(),
                complete.model_id()
            )));
        }
    }
    Ok(())
}

pub(super) fn account_matches_binding(
    account: &ManagedConnectionAccount,
    binding: &ManagedConnectionBinding,
) -> bool {
    let complete = binding.complete().binding();
    account.provider_id() == complete.provider_id() && account.account_id() == complete.account_id()
}

pub(super) fn binding_matches_selection(
    binding: &ManagedConnectionBinding,
    selection: &ModelSelection,
) -> bool {
    let complete = binding.complete().binding();
    complete.provider_id() == selection.provider()
        && complete.account_id() == selection.account()
        && complete.model_id() == selection.model()
}

fn reject_new_host_provider(provider_id: &ProviderId) -> Result<(), ModelServiceError> {
    if provider_id.as_str() == "host" {
        return Err(ModelServiceError::new(
            "new managed connections cannot use the reserved ProviderId host",
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
            "inconsistent managed display name for Provider {provider_id}"
        ))),
    }
}
