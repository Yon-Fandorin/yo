use std::collections::HashSet;

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use super::{
    OpenRouterDiscoveredModel, OpenRouterDiscoveryError, OpenRouterDiscoveryFailureKind,
    OpenRouterDiscoverySeed, failure, limit_failure,
};
use crate::{
    EffectiveModelBinding, EffectiveModelProfile, ModelCatalogEntry, ModelId, ModelProfileLayer,
    ModelServiceError, VersionedProfileId,
};

pub(super) const MAX_ROWS: usize = 4_096;
pub(super) const MAX_REMOTE_NAME_BYTES: usize = 96;

pub(super) fn normalize_catalog(
    seed: &OpenRouterDiscoverySeed,
    bytes: &[u8],
) -> Result<Vec<OpenRouterDiscoveredModel>, OpenRouterDiscoveryError> {
    let root: Value = serde_json::from_slice(bytes).map_err(|_| {
        failure(
            OpenRouterDiscoveryFailureKind::Protocol,
            "OpenRouter discovery response is not valid JSON",
        )
    })?;
    let rows = root.get("data").and_then(Value::as_array).ok_or_else(|| {
        failure(
            OpenRouterDiscoveryFailureKind::Protocol,
            "OpenRouter discovery response requires a data array",
        )
    })?;
    if rows.len() > MAX_ROWS {
        return Err(limit_failure(
            "OpenRouter discovery response contains more than 4096 model rows",
        ));
    }

    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        let Some(id) = object.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Ok(model_id) = ModelId::new(id.to_owned()) else {
            continue;
        };
        if !seen.insert(model_id.clone()) {
            continue;
        }

        let Some(architecture) = object.get("architecture").and_then(Value::as_object) else {
            continue;
        };
        let Some(input_modalities) = string_array(architecture.get("input_modalities")) else {
            continue;
        };
        let Some(output_modalities) = string_array(architecture.get("output_modalities")) else {
            continue;
        };
        let Some(supported) = string_array(object.get("supported_parameters")) else {
            continue;
        };
        if !input_modalities.contains(&"text")
            || !output_modalities.contains(&"text")
            || !supported.contains(&"tools")
        {
            continue;
        }

        let Some(input_limit) = optional_positive_u64(object.get("context_length")) else {
            continue;
        };
        let Some(output_limit) = optional_top_provider_limit(object.get("top_provider")) else {
            continue;
        };
        let display_name = accepted_display_name(object.get("name"), id);
        let entry = if let Some(authored) = seed.authored_models.get(&model_id) {
            let complete = authored
                .entry
                .complete_binding()
                .expect("authored discovery models are validated as complete");
            let profile = profile_with_remote_limits(
                complete.profile(),
                authored.input_token_limit.or(input_limit),
                authored.max_output_tokens.or(output_limit),
            )
            .ok();
            let Some(profile) = profile else {
                continue;
            };
            match ModelCatalogEntry::with_explicit_profile(
                complete.binding().clone(),
                authored.entry.provider_display_name().map(str::to_owned),
                authored.entry.account_display_name().map(str::to_owned),
                authored.entry.model_display_name().map(str::to_owned),
                profile,
            ) {
                Ok(entry) => entry,
                Err(_) => continue,
            }
        } else {
            let profile =
                profile_with_remote_limits(&seed.base_profile, input_limit, output_limit).ok();
            let Some(profile) = profile else {
                continue;
            };
            let binding = EffectiveModelBinding::new(
                seed.provider.clone(),
                seed.account.clone(),
                model_id,
                profile.api_dialect(),
                seed.endpoint.clone(),
            );
            match ModelCatalogEntry::with_explicit_profile(
                binding,
                seed.provider_display_name.clone(),
                seed.account_display_name.clone(),
                Some(display_name.clone()),
                profile,
            ) {
                Ok(entry) => entry,
                Err(_) => continue,
            }
        };
        models.push(OpenRouterDiscoveredModel {
            entry,
            display_name,
            reasoning: supported.contains(&"reasoning"),
        });
    }

    models.sort_by(|left, right| {
        search_key(left.display_name())
            .cmp(&search_key(right.display_name()))
            .then_with(|| {
                left.entry()
                    .binding()
                    .model_id()
                    .as_str()
                    .cmp(right.entry().binding().model_id().as_str())
            })
    });
    Ok(models)
}

pub(super) fn profile_with_remote_limits(
    base: &EffectiveModelProfile,
    input_limit: Option<u64>,
    output_limit: Option<u64>,
) -> Result<EffectiveModelProfile, ModelServiceError> {
    let layer = ModelProfileLayer::new(
        Some(base.api_dialect()),
        Some(VersionedProfileId::new(base.context().tokenizer_profile())?),
        Some(input_limit.unwrap_or(base.context().input_token_limit())),
        Some(output_limit.unwrap_or(base.context().max_output_tokens())),
        Some(base.reasoning_parameters().clone()),
        Some(base.optional_request_parameters().clone()),
        Some(base.tool_capability_policy().clone()),
        Some(base.verification_profile().clone()),
    );
    EffectiveModelProfile::resolve(None, &layer)
}

fn string_array(value: Option<&Value>) -> Option<Vec<&str>> {
    value?
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
}

fn optional_positive_u64(value: Option<&Value>) -> Option<Option<u64>> {
    match value {
        None | Some(Value::Null) => Some(None),
        Some(value) => value.as_u64().filter(|value| *value > 0).map(Some),
    }
}

fn optional_top_provider_limit(value: Option<&Value>) -> Option<Option<u64>> {
    match value {
        None | Some(Value::Null) => Some(None),
        Some(Value::Object(object)) => optional_positive_u64(object.get("max_completion_tokens")),
        Some(_) => None,
    }
}

fn accepted_display_name(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= MAX_REMOTE_NAME_BYTES)
        .unwrap_or(fallback)
        .to_owned()
}

fn search_key(value: &str) -> String {
    value.nfkc().flat_map(char::to_lowercase).collect()
}
