use std::{collections::BTreeMap, fmt};

use super::{
    AccountId, CatalogBinding, CompleteModelBinding, ModelCatalog, ModelCatalogEntry,
    ModelCatalogProvenance, ModelId, ProviderId,
};
use crate::{ManagedConnectionAccount, ManagedConnectionBinding};

impl ModelCatalog {
    /// Composes one fresh manual catalog with one captured managed snapshot.
    pub(crate) fn compose_managed(
        &self,
        accounts: &[ManagedConnectionAccount],
        bindings: &[ManagedConnectionBinding],
    ) -> Result<Self, BindingConflict> {
        let manual_provider_names = display_names_by_provider(&self.entries);
        let manual_account_names = display_names_by_account(&self.entries);
        let managed_provider_names = accounts
            .iter()
            .map(|account| {
                (
                    account.provider_id().clone(),
                    account.provider_display_name().map(str::to_owned),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let managed_account_names = accounts
            .iter()
            .map(|account| {
                (
                    (account.provider_id().clone(), account.account_id().clone()),
                    account.account_display_name().map(str::to_owned),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let mut entries = self.entries.clone();
        let mut positions = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry_coordinate(entry), index))
            .collect::<std::collections::HashMap<_, _>>();
        for managed in bindings {
            let coordinate = managed_coordinate(managed);
            if let Some(index) = positions.get(&coordinate).copied() {
                let manual = &mut entries[index];
                match manual.complete_binding() {
                    Some(complete) if complete == managed.complete() => {
                        manual.provenance = ModelCatalogProvenance::ManualAndManaged;
                        if manual.model_display_name.is_none() {
                            manual.model_display_name =
                                managed.model_display_name().map(str::to_owned);
                        }
                    },
                    Some(complete) => {
                        return Err(BindingConflict::new(
                            &coordinate,
                            complete_binding_differences(complete, managed.complete()),
                        ));
                    },
                    None => {
                        return Err(BindingConflict::new(&coordinate, vec!["resolved_profile"]));
                    },
                }
                continue;
            }

            let complete = managed.complete().clone();
            let index = entries.len();
            entries.push(ModelCatalogEntry {
                binding: CatalogBinding::Complete(complete),
                provider_display_name: None,
                account_display_name: None,
                model_display_name: managed.model_display_name().map(str::to_owned),
                provenance: ModelCatalogProvenance::Managed,
            });
            positions.insert(coordinate, index);
        }

        for entry in &mut entries {
            let provider_id = entry.binding().provider_id().clone();
            let account_id = entry.binding().account_id().clone();
            entry.provider_display_name = manual_provider_names
                .get(&provider_id)
                .cloned()
                .flatten()
                .or_else(|| managed_provider_names.get(&provider_id).cloned().flatten());
            let account_coordinate = (provider_id, account_id);
            entry.account_display_name = manual_account_names
                .get(&account_coordinate)
                .cloned()
                .flatten()
                .or_else(|| {
                    managed_account_names
                        .get(&account_coordinate)
                        .cloned()
                        .flatten()
                });
        }
        Ok(Self { entries })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindingConflict {
    provider: ProviderId,
    account: AccountId,
    model: ModelId,
    differing_fields: Vec<&'static str>,
}

impl BindingConflict {
    fn new(
        coordinate: &(ProviderId, AccountId, ModelId),
        differing_fields: Vec<&'static str>,
    ) -> Self {
        Self {
            provider: coordinate.0.clone(),
            account: coordinate.1.clone(),
            model: coordinate.2.clone(),
            differing_fields,
        }
    }

    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    #[must_use]
    pub fn differing_fields(&self) -> &[&'static str] {
        &self.differing_fields
    }
}

impl fmt::Display for BindingConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "BindingConflict for Provider {}, Account {}, Model {} differs in {}",
            self.provider,
            self.account,
            self.model,
            self.differing_fields.join(", ")
        )
    }
}

impl std::error::Error for BindingConflict {}

fn entry_coordinate(entry: &ModelCatalogEntry) -> (ProviderId, AccountId, ModelId) {
    let binding = entry.binding();
    (
        binding.provider_id().clone(),
        binding.account_id().clone(),
        binding.model_id().clone(),
    )
}

fn managed_coordinate(binding: &ManagedConnectionBinding) -> (ProviderId, AccountId, ModelId) {
    let binding = binding.complete().binding();
    (
        binding.provider_id().clone(),
        binding.account_id().clone(),
        binding.model_id().clone(),
    )
}

fn display_names_by_provider(
    entries: &[ModelCatalogEntry],
) -> BTreeMap<ProviderId, Option<String>> {
    entries
        .iter()
        .map(|entry| {
            (
                entry.binding().provider_id().clone(),
                entry.provider_display_name.clone(),
            )
        })
        .collect()
}

fn display_names_by_account(
    entries: &[ModelCatalogEntry],
) -> BTreeMap<(ProviderId, AccountId), Option<String>> {
    entries
        .iter()
        .map(|entry| {
            (
                (
                    entry.binding().provider_id().clone(),
                    entry.binding().account_id().clone(),
                ),
                entry.account_display_name.clone(),
            )
        })
        .collect()
}

fn complete_binding_differences(
    manual: &CompleteModelBinding,
    managed: &CompleteModelBinding,
) -> Vec<&'static str> {
    let mut differences = Vec::new();
    let manual_binding = manual.binding();
    let managed_binding = managed.binding();
    let manual_profile = manual.profile();
    let managed_profile = managed.profile();
    if manual_binding.endpoint() != managed_binding.endpoint() {
        differences.push("base_url");
    }
    if manual_binding.connector_id() != managed_binding.connector_id() {
        differences.push("connector");
    }
    if manual_profile.api_dialect() != managed_profile.api_dialect() {
        differences.push("api_dialect");
    }
    if manual_profile.context().tokenizer_profile() != managed_profile.context().tokenizer_profile()
    {
        differences.push("tokenizer_profile");
    }
    if manual_profile.context().input_token_limit() != managed_profile.context().input_token_limit()
    {
        differences.push("input_token_limit");
    }
    if manual_profile.context().max_output_tokens() != managed_profile.context().max_output_tokens()
    {
        differences.push("max_output_tokens");
    }
    if manual_profile.reasoning_parameters() != managed_profile.reasoning_parameters() {
        differences.push("reasoning_parameters");
    }
    if manual_profile.optional_request_parameters() != managed_profile.optional_request_parameters()
    {
        differences.push("optional_request_parameters");
    }
    if manual_profile.tool_capability_policy() != managed_profile.tool_capability_policy() {
        differences.push("tool_capability_policy");
    }
    if manual_profile.replay_profile() != managed_profile.replay_profile() {
        differences.push("replay_profile");
    }
    differences
}
