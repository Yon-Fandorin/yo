//! Durable managed binding identity and semantic resume comparison.

use serde::Deserialize;
use serde_json::{json, value::RawValue};
use yo_core::{
    BackendFailure, BackendFailureKind, BackendIdentity, CompleteModelBinding,
    EffectiveModelBinding, EffectiveModelProfile,
};

use super::failure;

const COMMAND_BINDING_SCHEMA: &str = "yo.managed-command-binding/v1";

pub(super) fn native_binding_identity(
    binding: &EffectiveModelBinding,
    profile: Option<&EffectiveModelProfile>,
    execution_manifest_digest: Option<&str>,
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
            if let Some(image_profile) = profile.image_input_profile() {
                value["image_input_profile"] = json!(image_profile.as_str());
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
    let identity = if let Some(digest) = execution_manifest_digest {
        if !valid_manifest_digest(digest) {
            return Err(invalid_command_binding(
                "execution manifest digest is invalid",
            ));
        }
        let model_binding: serde_json::Value =
            serde_json::from_str(&value).expect("generated model binding is valid JSON");
        BackendIdentity::new(
            COMMAND_BINDING_SCHEMA,
            json!({
                "model_binding": { "schema": schema, "value": model_binding },
                "execution_manifest_digest": digest,
            })
            .to_string(),
        )
    } else {
        BackendIdentity::new(schema, value)
    };
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
        COMMAND_BINDING_SCHEMA => match (
            decode_binding_identity(current),
            decode_binding_identity(durable),
        ) {
            (Ok((current, current_digest)), Ok((durable, durable_digest))) => {
                current_digest == durable_digest
                    && semantically_equal_native_binding_identity(&current, &durable)
            },
            _ => false,
        },
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

/// Decodes the managed identity while retaining the original nested JSON number spellings.
pub(super) fn decode_binding_identity(
    identity: &BackendIdentity,
) -> Result<(BackendIdentity, Option<String>), BackendFailure> {
    if !identity.is_valid() {
        return Err(invalid_command_binding(
            "managed binding exceeds its durable boundary",
        ));
    }
    if identity.schema() != COMMAND_BINDING_SCHEMA {
        if !semantically_equal_native_binding_identity(identity, identity) {
            return Err(invalid_command_binding("managed model binding is invalid"));
        }
        return Ok((identity.clone(), None));
    }
    let wrapper: CommandBinding<'_> = decode_object(identity.value())
        .map_err(|_| invalid_command_binding("managed command binding envelope is invalid"))?;
    if !valid_manifest_digest(&wrapper.execution_manifest_digest) {
        return Err(invalid_command_binding(
            "execution manifest digest is invalid",
        ));
    }
    let model_binding: ModelBinding<'_> = decode_object(wrapper.model_binding.get())
        .map_err(|_| invalid_command_binding("nested model binding envelope is invalid"))?;
    let value = model_binding.value.get();
    if !value.trim_start().starts_with('{') {
        return Err(invalid_command_binding(
            "nested model binding value must be an object",
        ));
    }
    match model_binding.schema.as_str() {
        "yo.model-binding/v1" => {
            // The old top-level decoder remains permissive; only the new envelope closes it.
            let _: LegacyNativeBindingIdentity = serde_json::from_str(value)
                .map_err(|_| invalid_command_binding("nested legacy binding is invalid"))?;
            let shape: serde_json::Value = serde_json::from_str(value)
                .map_err(|_| invalid_command_binding("nested legacy binding is invalid"))?;
            if !shape.as_object().is_some_and(|fields| {
                fields.len() == 6
                    && fields.keys().all(|key| {
                        matches!(
                            key.as_str(),
                            "provider"
                                | "account"
                                | "model"
                                | "connector"
                                | "api_dialect"
                                | "base_url"
                        )
                    })
            }) {
                return Err(invalid_command_binding(
                    "nested legacy binding has unknown fields",
                ));
            }
        },
        "yo.complete-model-binding/v1" => {
            CompleteModelBinding::from_durable_json(value)
                .map_err(|_| invalid_command_binding("nested complete binding is invalid"))?;
        },
        _ => {
            return Err(invalid_command_binding(
                "nested managed binding schema is unsupported",
            ));
        },
    }
    Ok((
        BackendIdentity::new(model_binding.schema, value),
        Some(wrapper.execution_manifest_digest),
    ))
}

pub(super) fn same_execution_manifest(
    current: &BackendIdentity,
    durable: &BackendIdentity,
) -> bool {
    if current.schema() != COMMAND_BINDING_SCHEMA && durable.schema() != COMMAND_BINDING_SCHEMA {
        return true;
    }
    match (
        decode_binding_identity(current),
        decode_binding_identity(durable),
    ) {
        (Ok((_, Some(current))), Ok((_, Some(durable)))) => current == durable,
        _ => false,
    }
}

fn valid_manifest_digest(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn invalid_command_binding(message: &str) -> BackendFailure {
    failure(BackendFailureKind::Initialization, message)
}

fn decode_object<'a, T: Deserialize<'a>>(value: &'a str) -> Result<T, ()> {
    if !value.trim_start().starts_with('{') {
        return Err(());
    }
    serde_json::from_str(value).map_err(|_| ())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandBinding<'a> {
    #[serde(borrow)]
    model_binding: &'a RawValue,
    execution_manifest_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelBinding<'a> {
    schema: String,
    #[serde(borrow)]
    value: &'a RawValue,
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
