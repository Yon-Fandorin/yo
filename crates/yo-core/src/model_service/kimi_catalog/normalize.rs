use std::collections::HashSet;

use serde_json::{Value, json};

use super::{
    KimiCatalogAvailability, KimiCatalogDisabledReason, KimiCatalogError, KimiCatalogFailureKind,
    KimiCatalogModel, KimiCatalogSeed, failure, limit_failure,
};
use crate::{
    ApiDialect, EffectiveModelBinding, EffectiveModelProfile, ModelCatalogEntry, ModelId,
    ModelProfileLayer, ModelProfileParameters, VersionedProfileId,
};

const MAX_ROWS: usize = 4_096;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn normalize_catalog(
    seed: &KimiCatalogSeed,
    bytes: &[u8],
) -> Result<Vec<KimiCatalogModel>, KimiCatalogError> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(limit_failure(
            "Kimi catalog response exceeds the 8 MiB snapshot limit",
        ));
    }
    let root: Value = serde_json::from_slice(bytes).map_err(|_| {
        failure(
            KimiCatalogFailureKind::Protocol,
            "Kimi catalog response is not valid JSON",
        )
    })?;
    if root.get("object").and_then(Value::as_str) != Some("list") {
        return Err(failure(
            KimiCatalogFailureKind::Protocol,
            "Kimi catalog response requires object list",
        ));
    }
    let rows = root.get("data").and_then(Value::as_array).ok_or_else(|| {
        failure(
            KimiCatalogFailureKind::Protocol,
            "Kimi catalog response requires a data array",
        )
    })?;
    if rows.len() > MAX_ROWS {
        return Err(limit_failure(
            "Kimi catalog response contains more than 4096 model rows",
        ));
    }

    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        if object.get("object").and_then(Value::as_str) != Some("model") {
            continue;
        }
        let Some(id) = object.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Ok(model_id) = ModelId::new(id.to_owned()) else {
            continue;
        };
        if !seen.insert(model_id.clone()) {
            continue;
        }
        let context = positive_u64(object.get("context_length"));
        let supports_reasoning = optional_bool(object.get("supports_reasoning"));
        let (profile, availability, reasoning, recommended, high_speed) =
            overlay(model_id.as_str(), context, supports_reasoning);
        let entry = profile.and_then(|profile| build_entry(seed, &model_id, profile).ok());
        let availability =
            if entry.is_none() && matches!(availability, KimiCatalogAvailability::Enabled) {
                KimiCatalogAvailability::Disabled(KimiCatalogDisabledReason::ProfileUnavailable)
            } else {
                availability
            };
        models.push(KimiCatalogModel {
            provider: seed.provider.clone(),
            account: seed.account.clone(),
            model_id,
            entry: matches!(availability, KimiCatalogAvailability::Enabled)
                .then_some(entry)
                .flatten(),
            input_limit: context,
            output_limit: profile_output_limit(profile),
            reasoning,
            recommended,
            high_speed,
            availability,
        });
    }
    models.sort_by(|left, right| {
        crate::normalized_search_key(left.model_id().as_str())
            .cmp(&crate::normalized_search_key(right.model_id().as_str()))
            .then_with(|| left.model_id().cmp(right.model_id()))
    });
    Ok(models)
}

#[derive(Clone, Copy)]
enum OverlayProfile {
    K3 { input: u64 },
    K27 { input: u64 },
    K26 { input: u64 },
}

fn overlay(
    model: &str,
    context: Option<u64>,
    reasoning: Option<bool>,
) -> (
    Option<OverlayProfile>,
    KimiCatalogAvailability,
    Option<bool>,
    bool,
    bool,
) {
    let profile = match (model, context) {
        ("kimi-k3", Some(input)) if (131_073..=1_048_576).contains(&input) => {
            Some(OverlayProfile::K3 { input })
        },
        ("kimi-k2.7-code" | "kimi-k2.7-code-highspeed", Some(input))
            if (32_769..=262_144).contains(&input) =>
        {
            Some(OverlayProfile::K27 { input })
        },
        ("kimi-k2.6", Some(input)) if (32_769..=262_144).contains(&input) => {
            Some(OverlayProfile::K26 { input })
        },
        _ => None,
    };
    let availability = if model == "kimi-k2.5" {
        KimiCatalogAvailability::Disabled(KimiCatalogDisabledReason::ProviderRetirement)
    } else if matches!(
        model,
        "kimi-k3" | "kimi-k2.7-code" | "kimi-k2.7-code-highspeed"
    ) && reasoning == Some(false)
    {
        KimiCatalogAvailability::Disabled(KimiCatalogDisabledReason::CapabilityConflict)
    } else if profile.is_some() {
        KimiCatalogAvailability::Enabled
    } else {
        KimiCatalogAvailability::Disabled(KimiCatalogDisabledReason::ProfileUnavailable)
    };
    let effective_reasoning = match profile {
        Some(OverlayProfile::K3 { .. } | OverlayProfile::K27 { .. }) => Some(true),
        Some(OverlayProfile::K26 { .. }) => reasoning,
        None => reasoning,
    };
    (
        profile,
        availability,
        effective_reasoning,
        model == "kimi-k3",
        model == "kimi-k2.7-code-highspeed",
    )
}

fn build_entry(
    seed: &KimiCatalogSeed,
    model_id: &ModelId,
    profile: OverlayProfile,
) -> Result<ModelCatalogEntry, crate::ModelServiceError> {
    let (input, output, reasoning, optional, replay) = match profile {
        OverlayProfile::K3 { input } => (
            input,
            131_072,
            json!({"effort": "max"}),
            json!({}),
            "kimi-private-local-plaintext/v1",
        ),
        OverlayProfile::K27 { input } => (
            input,
            32_768,
            json!({}),
            json!({"thinking": {"type": "enabled", "keep": "all"}}),
            "kimi-private-local-plaintext/v1",
        ),
        OverlayProfile::K26 { input } => (
            input,
            32_768,
            json!({}),
            json!({"thinking": {"type": "disabled"}}),
            "semantic-only/v1",
        ),
    };
    let layer = ModelProfileLayer::new(
        Some(ApiDialect::KimiChatCompletions),
        Some(profile_id("utf8-bytes/v1")),
        Some(input),
        Some(output),
        Some(parameters(reasoning)),
        Some(parameters(optional)),
        Some(profile_id("local-tools/v1")),
        Some(profile_id("semantic-terminal/v1")),
    )
    .with_replay_profile(Some(profile_id(replay)));
    let profile = EffectiveModelProfile::resolve(None, &layer)?;
    let binding = EffectiveModelBinding::new(
        seed.provider.clone(),
        seed.account.clone(),
        model_id.clone(),
        profile.api_dialect(),
        seed.endpoint.clone(),
    );
    ModelCatalogEntry::with_explicit_profile(
        binding,
        seed.provider_display_name.clone(),
        seed.account_display_name.clone(),
        Some(model_id.as_str().to_owned()),
        profile,
    )
}

fn parameters(value: Value) -> ModelProfileParameters {
    serde_json::from_value(value).expect("the closed Kimi overlay uses valid profile values")
}

fn profile_id(value: &str) -> VersionedProfileId {
    VersionedProfileId::new(value).expect("the closed Kimi overlay profile ID is valid")
}

fn positive_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64).filter(|value| *value > 0)
}

fn optional_bool(value: Option<&Value>) -> Option<bool> {
    value.and_then(Value::as_bool)
}

fn profile_output_limit(profile: Option<OverlayProfile>) -> Option<u64> {
    profile.map(|profile| match profile {
        OverlayProfile::K3 { .. } => 131_072,
        OverlayProfile::K27 { .. } | OverlayProfile::K26 { .. } => 32_768,
    })
}
