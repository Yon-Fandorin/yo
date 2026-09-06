use yo_core::{
    AccountId, ConnectionAccount, ConnectionSnapshot, ModelCatalogEntry, ModelSelection, ProviderId,
};
use yo_provider_kimi::{KimiCatalogSeed, discover_kimi_models};
use yo_provider_openrouter::{OpenRouterDiscoverySeed, discover_openrouter_models};
use yo_provider_qwencloud::QwenCloudCatalogSeed;

use crate::{
    AppError,
    command::connect::{input::ExternalConnectInput, picker::ModelPickerItem},
    state::connection::selection_for_binding,
};

pub(super) fn discover_openrouter_and_select<I>(
    seed: &OpenRouterDiscoverySeed,
    candidate: &yo_core::ApiCredential,
    input: &mut I,
) -> Result<Option<ModelCatalogEntry>, AppError>
where
    I: ExternalConnectInput,
{
    let models = discover_openrouter_models(seed, candidate)
        .map_err(|error| AppError::single("discovering OpenRouter account models", error))?;
    if models.is_empty() {
        return Err(AppError::message(
            "OpenRouter discovery returned no valid ModelId",
        ));
    }
    let choices = models
        .iter()
        .map(ModelPickerItem::from_openrouter)
        .collect::<Vec<_>>();
    let Some(selected) = input.select_model(&choices)? else {
        return Ok(None);
    };
    models
        .get(selected)
        .and_then(|model| model.entry().cloned())
        .map(Some)
        .ok_or_else(|| {
            AppError::message(
                "the OpenRouter picker returned an invalid or disabled model selection",
            )
        })
}

pub(super) fn discover_kimi_and_select<I>(
    seed: &KimiCatalogSeed,
    candidate: &yo_core::ApiCredential,
    input: &mut I,
) -> Result<Option<ModelCatalogEntry>, AppError>
where
    I: ExternalConnectInput,
{
    let models = discover_kimi_models(seed, candidate)
        .map_err(|error| AppError::single("discovering Kimi account models", error))?;
    if models.is_empty() {
        return Err(AppError::message("Kimi catalog returned no valid ModelId"));
    }
    let choices = models
        .iter()
        .map(ModelPickerItem::from_kimi)
        .collect::<Vec<_>>();
    let Some(selected) = input.select_model(&choices)? else {
        return Ok(None);
    };
    models
        .get(selected)
        .and_then(|model| model.entry().cloned())
        .map(Some)
        .ok_or_else(|| {
            AppError::message("the Kimi picker returned an invalid or disabled model selection")
        })
}

pub(super) fn select_qwencloud(
    seed: &QwenCloudCatalogSeed,
    input: &mut impl ExternalConnectInput,
) -> Result<Option<ModelCatalogEntry>, AppError> {
    let choices = seed
        .models()
        .iter()
        .map(ModelPickerItem::from_qwencloud)
        .collect::<Vec<_>>();
    let Some(selected) = input.select_model(&choices)? else {
        return Ok(None);
    };
    seed.models()
        .get(selected)
        .and_then(|model| model.entry().cloned())
        .map(Some)
        .ok_or_else(|| {
            AppError::message(
                "the QwenCloud picker returned an invalid or disabled model selection",
            )
        })
}

pub(super) fn selected_entry(
    snapshot: &ConnectionSnapshot,
    reference: &str,
) -> Result<ModelCatalogEntry, AppError> {
    let catalog = snapshot
        .model_catalog()
        .map_err(|error| AppError::single("reading the stored model catalog", error))?;
    if let Some(entry) = catalog
        .entries()
        .iter()
        .find(|entry| selection_for(entry).canonical_reference() == reference)
        .cloned()
    {
        return Ok(entry);
    }
    for stored_seed in snapshot.catalog_seeds() {
        let Some(seed) = QwenCloudCatalogSeed::from_connection_seed(stored_seed)
            .map_err(|error| AppError::single("reading the stored QwenCloud seed", error))?
        else {
            continue;
        };
        if let Some(row) = seed.models().iter().find(|row| {
            ModelSelection::new(
                row.provider().clone(),
                row.account().clone(),
                row.model_id().clone(),
            )
            .canonical_reference()
                == reference
        }) {
            return row.entry().cloned().ok_or_else(|| {
                let reason = match row.availability() {
                    yo_provider_qwencloud::QwenCloudCatalogAvailability::Enabled => {
                        "invalid catalog row"
                    },
                    yo_provider_qwencloud::QwenCloudCatalogAvailability::Disabled(reason) => {
                        reason.as_str()
                    },
                };
                AppError::message(format!(
                    "QwenCloud catalog model {reference:?} is disabled: {reason}"
                ))
            });
        }
    }
    Err(AppError::message(format!(
        "external connect target {reference:?} is not an exact stored Provider:Account:Model reference; import its definition with yo connect --from"
    )))
}

pub(super) fn catalog_pair(
    snapshot: &ConnectionSnapshot,
    reference: &str,
) -> Result<Option<(ProviderId, AccountId)>, AppError> {
    if let Some(account) = snapshot.accounts().iter().find(|account| {
        matches!(
            account.provider_id().as_str(),
            "openrouter" | "qwencloud" | "kimi"
        ) && account.canonical_reference() == reference
            && snapshot.catalog_seeds().iter().any(|seed| {
                seed.provider() == account.provider_id() && seed.account() == account.account_id()
            })
    }) {
        return Ok(Some((
            account.provider_id().clone(),
            account.account_id().clone(),
        )));
    }
    let mut segments = reference.split(':');
    let Some(provider) = segments.next() else {
        return Ok(None);
    };
    let Some(_account) = segments.next() else {
        return Ok(None);
    };
    if segments.next().is_some() {
        return Ok(None);
    }
    if !matches!(provider, "openrouter" | "qwencloud" | "kimi") {
        return Err(AppError::message(format!(
            "two-part external connect target {reference:?} is unsupported; only configured openrouter:Account or kimi:Account discovery and qwencloud:Account catalog selection are admitted"
        )));
    }
    Err(AppError::message(format!(
        "catalog target {reference:?} is not an exact stored Provider:Account seed"
    )))
}

pub(super) fn looks_like_two_part_target(reference: &str) -> bool {
    let mut segments = reference.split(':');
    segments.next().is_some() && segments.next().is_some() && segments.next().is_none()
}

pub(super) fn stored_account_reference(
    snapshot: &ConnectionSnapshot,
    provider: &ProviderId,
    account: &AccountId,
) -> Result<String, AppError> {
    snapshot
        .accounts()
        .iter()
        .find(|stored| stored.provider_id() == provider && stored.account_id() == account)
        .map(ConnectionAccount::canonical_reference)
        .ok_or_else(|| {
            AppError::message(format!(
                "stored Provider {provider} and Account {account} has no account definition"
            ))
        })
}

pub(super) fn selection_for(entry: &ModelCatalogEntry) -> ModelSelection {
    selection_for_binding(entry.binding())
}
