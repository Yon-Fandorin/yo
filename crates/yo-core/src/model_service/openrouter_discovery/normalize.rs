use std::collections::{BTreeSet, HashSet};

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use super::{
    OpenRouterDisabledReason, OpenRouterDiscoveredModel, OpenRouterDiscoveryError,
    OpenRouterDiscoveryFailureKind, OpenRouterDiscoverySeed, OpenRouterModelAvailability,
    OpenRouterModelCapabilities, failure, limit_failure,
};
use crate::{
    EffectiveModelBinding, EffectiveModelProfile, ModelCatalogEntry, ModelId, ModelProfileLayer,
    ModelServiceError, VersionedProfileId,
};

pub(super) const MAX_ROWS: usize = 4_096;
pub(super) const MAX_REMOTE_NAME_BYTES: usize = 96;

const LOCAL_TOOLS_PROFILE: &str = "local-tools/v1";
const NO_TOOLS_PROFILE: &str = "no-tools/v1";

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

        let display_name = accepted_display_name(object.get("name"), id);
        let capabilities = capability_evidence(object);
        let configured_profile = seed
            .authored_models
            .get(&model_id)
            .map(|authored| {
                authored
                    .entry
                    .complete_binding()
                    .expect("authored discovery models are validated as complete")
                    .profile()
            })
            .unwrap_or(&seed.base_profile);
        let effective_tool_policy = capabilities
            .as_ref()
            .and_then(|capabilities| intersect_tool_policy(configured_profile, capabilities));
        let reasoning = capabilities
            .as_ref()
            .map(|capabilities| capabilities.supported_parameters.contains("reasoning"));
        let limits = resolved_limits(
            configured_profile,
            seed.authored_models.get(&model_id),
            remote_positive_u64(object.get("context_length")),
            remote_top_provider_limit(object.get("top_provider")),
        );
        let entry = match (limits, effective_tool_policy.as_ref()) {
            (Ok((input_limit, output_limit)), Some(tool_policy)) => build_entry(
                seed,
                &model_id,
                &display_name,
                input_limit,
                output_limit,
                tool_policy,
            )
            .ok(),
            _ => None,
        };
        let availability = availability(&capabilities, configured_profile, entry.as_ref());
        let entry = matches!(availability, OpenRouterModelAvailability::Enabled)
            .then_some(entry)
            .flatten();
        let (input_limit, output_limit) = entry
            .as_ref()
            .map(|entry| {
                (
                    Some(entry.context().input_token_limit()),
                    entry.context().max_output_tokens(),
                )
            })
            .unwrap_or_else(|| match limits {
                Ok((input, output)) => (Some(input), output),
                Err(()) => (None, None),
            });
        models.push(OpenRouterDiscoveredModel {
            provider: seed.provider.clone(),
            account: seed.account.clone(),
            model_id,
            entry,
            display_name,
            capabilities,
            input_limit,
            output_limit,
            effective_tool_policy,
            reasoning,
            availability,
        });
    }

    models.sort_by(|left, right| {
        search_key(left.display_name())
            .cmp(&search_key(right.display_name()))
            .then_with(|| left.model_id().as_str().cmp(right.model_id().as_str()))
    });
    Ok(models)
}

fn capability_evidence(
    object: &serde_json::Map<String, Value>,
) -> Option<OpenRouterModelCapabilities> {
    let architecture = object.get("architecture")?.as_object()?;
    Some(OpenRouterModelCapabilities {
        input_modalities: string_set(architecture.get("input_modalities"))?,
        output_modalities: string_set(architecture.get("output_modalities"))?,
        supported_parameters: string_set(object.get("supported_parameters"))?,
    })
}

fn string_set(value: Option<&Value>) -> Option<BTreeSet<String>> {
    value?
        .as_array()?
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect()
}

fn intersect_tool_policy(
    profile: &EffectiveModelProfile,
    capabilities: &OpenRouterModelCapabilities,
) -> Option<VersionedProfileId> {
    match profile.tool_capability_policy().as_str() {
        NO_TOOLS_PROFILE => Some(
            VersionedProfileId::new(NO_TOOLS_PROFILE)
                .expect("the closed no-tools policy is a valid profile ID"),
        ),
        LOCAL_TOOLS_PROFILE
            if capabilities.supported_parameters.contains("tools")
                && capabilities.supported_parameters.contains("tool_choice") =>
        {
            Some(
                VersionedProfileId::new(LOCAL_TOOLS_PROFILE)
                    .expect("the closed local-tools policy is a valid profile ID"),
            )
        },
        LOCAL_TOOLS_PROFILE => Some(
            VersionedProfileId::new(NO_TOOLS_PROFILE)
                .expect("the closed no-tools policy is a valid profile ID"),
        ),
        _ => None,
    }
}

fn availability(
    capabilities: &Option<OpenRouterModelCapabilities>,
    configured_profile: &EffectiveModelProfile,
    entry: Option<&ModelCatalogEntry>,
) -> OpenRouterModelAvailability {
    let Some(capabilities) = capabilities else {
        return OpenRouterModelAvailability::Disabled(
            OpenRouterDisabledReason::CapabilitiesUnavailable,
        );
    };
    if !capabilities.input_modalities.contains("text") {
        return OpenRouterModelAvailability::Disabled(
            OpenRouterDisabledReason::TextInputUnsupported,
        );
    }
    if !capabilities.output_modalities.contains("text") {
        return OpenRouterModelAvailability::Disabled(
            OpenRouterDisabledReason::TextOutputUnsupported,
        );
    }
    if !matches!(
        configured_profile.tool_capability_policy().as_str(),
        LOCAL_TOOLS_PROFILE | NO_TOOLS_PROFILE
    ) {
        return OpenRouterModelAvailability::Disabled(
            OpenRouterDisabledReason::ToolPolicyUnsupported,
        );
    }
    if entry.is_none() {
        return OpenRouterModelAvailability::Disabled(OpenRouterDisabledReason::ProfileUnavailable);
    }
    OpenRouterModelAvailability::Enabled
}

fn resolved_limits(
    profile: &EffectiveModelProfile,
    authored: Option<&super::OpenRouterAuthoredModel>,
    input_limit: RemoteLimit,
    output_limit: RemoteLimit,
) -> Result<(u64, Option<u64>), ()> {
    Ok((
        resolve_limit(
            authored.and_then(|authored| authored.input_token_limit),
            input_limit,
            profile.context().input_token_limit(),
        )?,
        resolve_optional_limit(
            authored.and_then(|authored| authored.max_output_tokens),
            output_limit,
            profile.context().max_output_tokens(),
        )?,
    ))
}

fn resolve_limit(authored: Option<u64>, remote: RemoteLimit, base: u64) -> Result<u64, ()> {
    match remote {
        RemoteLimit::Invalid => Err(()),
        RemoteLimit::Missing => Ok(authored.unwrap_or(base)),
        RemoteLimit::Value(value) => Ok(authored.unwrap_or(value)),
    }
}

fn resolve_optional_limit(
    authored: Option<u64>,
    remote: RemoteLimit,
    base: Option<u64>,
) -> Result<Option<u64>, ()> {
    match remote {
        RemoteLimit::Invalid => Err(()),
        RemoteLimit::Missing => Ok(authored.or(base)),
        RemoteLimit::Value(value) => Ok(Some(authored.unwrap_or(value))),
    }
}

fn build_entry(
    seed: &OpenRouterDiscoverySeed,
    model_id: &ModelId,
    remote_display_name: &str,
    input_limit: u64,
    output_limit: Option<u64>,
    tool_policy: &VersionedProfileId,
) -> Result<ModelCatalogEntry, ModelServiceError> {
    if let Some(authored) = seed.authored_models.get(model_id) {
        let complete = authored
            .entry
            .complete_binding()
            .expect("authored discovery models are validated as complete");
        let profile = profile_with_remote_values(
            complete.profile(),
            input_limit,
            output_limit,
            tool_policy.as_str(),
        )?;
        return ModelCatalogEntry::with_explicit_profile(
            complete.binding().clone(),
            authored.entry.provider_display_name().map(str::to_owned),
            authored.entry.account_display_name().map(str::to_owned),
            authored.entry.model_display_name().map(str::to_owned),
            profile,
        );
    }

    let profile = profile_with_remote_values(
        &seed.base_profile,
        input_limit,
        output_limit,
        tool_policy.as_str(),
    )?;
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
        Some(remote_display_name.to_owned()),
        profile,
    )
}

#[cfg(test)]
pub(super) fn profile_with_remote_limits(
    base: &EffectiveModelProfile,
    input_limit: Option<u64>,
    output_limit: Option<u64>,
) -> Result<EffectiveModelProfile, ModelServiceError> {
    profile_with_remote_values(
        base,
        input_limit.unwrap_or(base.context().input_token_limit()),
        output_limit.or(base.context().max_output_tokens()),
        base.tool_capability_policy().as_str(),
    )
}

fn profile_with_remote_values(
    base: &EffectiveModelProfile,
    input_limit: u64,
    output_limit: Option<u64>,
    tool_policy: &str,
) -> Result<EffectiveModelProfile, ModelServiceError> {
    let layer = ModelProfileLayer::new(
        Some(base.api_dialect()),
        Some(VersionedProfileId::new(base.context().tokenizer_profile())?),
        Some(input_limit),
        output_limit,
        Some(base.reasoning_parameters().clone()),
        Some(base.optional_request_parameters().clone()),
        Some(VersionedProfileId::new(tool_policy)?),
    );
    EffectiveModelProfile::resolve(None, &layer)
}

#[derive(Clone, Copy)]
enum RemoteLimit {
    Missing,
    Value(u64),
    Invalid,
}

fn remote_positive_u64(value: Option<&Value>) -> RemoteLimit {
    match value {
        None | Some(Value::Null) => RemoteLimit::Missing,
        Some(value) => value
            .as_u64()
            .filter(|value| *value > 0)
            .map_or(RemoteLimit::Invalid, RemoteLimit::Value),
    }
}

fn remote_top_provider_limit(value: Option<&Value>) -> RemoteLimit {
    match value {
        None | Some(Value::Null) => RemoteLimit::Missing,
        Some(Value::Object(object)) => remote_positive_u64(object.get("max_completion_tokens")),
        Some(_) => RemoteLimit::Invalid,
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
