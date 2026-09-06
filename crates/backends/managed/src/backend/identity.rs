//! Durable managed binding identity and semantic resume comparison.

use serde::Deserialize;
use serde_json::json;
use yo_core::{
    BackendFailure, BackendFailureKind, BackendIdentity, CompleteModelBinding,
    EffectiveModelBinding, EffectiveModelProfile,
};

use super::failure;

pub(super) fn native_binding_identity(
    binding: &EffectiveModelBinding,
    profile: Option<&EffectiveModelProfile>,
) -> Result<BackendIdentity, BackendFailure> {
    let (schema, value) = match profile {
        Some(profile) => {
            let mut value = json!({
                "provider": binding.provider_id().as_str(),
                "account": binding.account_id().as_str(),
                "model": binding.model_id().as_str(),
                "connector": binding.connector_id().as_str(),
                "base_url": binding.endpoint().as_str(),
                "api_dialect": profile.api_dialect().as_str(),
                "tokenizer_profile": profile.context().tokenizer_profile(),
                "input_token_limit": profile.context().input_token_limit(),
                "reasoning_parameters": profile.reasoning_parameters(),
                "optional_request_parameters": profile.optional_request_parameters(),
                "tool_capability_policy": profile.tool_capability_policy().as_str(),
            });
            if let Some(max_output_tokens) = profile.context().max_output_tokens() {
                value["max_output_tokens"] = json!(max_output_tokens);
            }
            if profile.replay_profile().as_str() != yo_core::SEMANTIC_REPLAY_PROFILE {
                value["replay_profile"] = json!(profile.replay_profile().as_str());
            }
            ("yo.complete-model-binding/v1", value.to_string())
        },
        None => (
            "yo.model-binding/v1",
            json!({
                "provider": binding.provider_id().as_str(),
                "account": binding.account_id().as_str(),
                "model": binding.model_id().as_str(),
                "connector": binding.connector_id().as_str(),
                "api_dialect": binding.api_dialect().as_str(),
                "base_url": binding.endpoint().as_str(),
            })
            .to_string(),
        ),
    };
    let identity = BackendIdentity::new(schema, value);
    if !identity.is_valid() {
        return Err(failure(
            BackendFailureKind::Initialization,
            "complete native model binding exceeds the durable identity boundary",
        ));
    }
    Ok(identity)
}

pub(super) fn semantically_equal_native_binding_identity(
    current: &BackendIdentity,
    durable: &BackendIdentity,
) -> bool {
    if current.schema() != durable.schema() {
        return false;
    }
    match current.schema() {
        "yo.model-binding/v1" => match (
            serde_json::from_str::<LegacyNativeBindingIdentity>(current.value()),
            serde_json::from_str::<LegacyNativeBindingIdentity>(durable.value()),
        ) {
            (Ok(current), Ok(durable)) => current == durable,
            _ => false,
        },
        "yo.complete-model-binding/v1" => match (
            CompleteModelBinding::from_durable_json(current.value()),
            CompleteModelBinding::from_durable_json(durable.value()),
        ) {
            (Ok(current), Ok(durable)) => current == durable,
            _ => false,
        },
        _ => false,
    }
}

#[derive(Deserialize, Eq, PartialEq)]
struct LegacyNativeBindingIdentity {
    provider: String,
    account: String,
    model: String,
    connector: String,
    api_dialect: String,
    base_url: String,
}
