use serde_json::{Value, json};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountId,
    BackendFailure, BackendFailureKind, ProviderId,
};

pub(super) const PROTOCOL_VERSION: u64 = 1;
const MAX_REQUEST_ID_BYTES: usize = 4096;

#[derive(Debug)]
pub(super) enum Incoming {
    Response {
        id: u64,
        result: Value,
    },
    ResponseError {
        id: u64,
        code: i64,
        message: String,
    },
    Notification {
        method: String,
        params: Value,
    },
    ServerRequest {
        id: Value,
        method: String,
        params: Value,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct InitializeResult {
    pub(super) agent_name: String,
    pub(super) agent_version: String,
    pub(super) auth_methods: Vec<String>,
    pub(super) load_session: bool,
    pub(super) current_model_id: Option<String>,
    pub(super) available_models: Vec<(String, String)>,
}

pub(super) fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

pub(super) fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

pub(super) fn server_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub(super) fn server_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

pub(super) fn classify(value: Value) -> Result<Incoming, BackendFailure> {
    let object = value
        .as_object()
        .ok_or_else(|| protocol_failure("Grok ACP message must be a JSON object"))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(protocol_failure(
            "Grok ACP message does not declare JSON-RPC 2.0",
        ));
    }
    let method = object.get("method").and_then(Value::as_str);
    let id = object.get("id");

    match (method, id) {
        (Some(method), Some(id)) => {
            if !(id.is_null()
                || id
                    .as_str()
                    .is_some_and(|value| value.len() <= MAX_REQUEST_ID_BYTES)
                || id.as_i64().is_some())
            {
                return Err(protocol_failure(
                    "request id from Grok ACP must be a string, signed integer, or null",
                ));
            }
            Ok(Incoming::ServerRequest {
                id: id.clone(),
                method: method.to_owned(),
                params: object.get("params").cloned().unwrap_or(Value::Null),
            })
        },
        (Some(method), None) => Ok(Incoming::Notification {
            method: method.to_owned(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(id)) => {
            let id = id.as_u64().ok_or_else(|| {
                // Report the wire shape without exposing an arbitrary host-provided ID.
                let kind = match id {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "non-unsigned number",
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                };
                protocol_failure(format!(
                    "response id from Grok ACP must be an unsigned integer (received {kind})"
                ))
            })?;
            match (object.get("result"), object.get("error")) {
                (Some(result), None) => Ok(Incoming::Response {
                    id,
                    result: result.clone(),
                }),
                (None, Some(error)) => {
                    let error = error.as_object().ok_or_else(|| {
                        protocol_failure("Grok ACP error response is not an object")
                    })?;
                    let code = error.get("code").and_then(Value::as_i64).ok_or_else(|| {
                        protocol_failure("Grok ACP error response has no numeric code")
                    })?;
                    let message =
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                protocol_failure("Grok ACP error response has no message")
                            })?;
                    Ok(Incoming::ResponseError {
                        id,
                        code,
                        message: message.to_owned(),
                    })
                },
                _ => Err(protocol_failure(
                    "Grok ACP response must contain exactly one of result or error",
                )),
            }
        },
        (None, None) => Err(protocol_failure(
            "Grok ACP message has neither method nor id",
        )),
    }
}

pub(super) fn decode_initialize(result: Value) -> Result<InitializeResult, BackendFailure> {
    let version = result
        .get("protocolVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| initialization_failure("Grok initialize response has no protocolVersion"))?;
    if version != PROTOCOL_VERSION {
        return Err(initialization_failure(format!(
            "unsupported Grok ACP protocol version {version}; yo requires version {PROTOCOL_VERSION}"
        )));
    }
    let auth_method_values = result
        .get("authMethods")
        .and_then(Value::as_array)
        .ok_or_else(|| initialization_failure("Grok initialize response has no authMethods"))?;
    let mut auth_methods = Vec::with_capacity(auth_method_values.len());
    for method in auth_method_values {
        let id = method.get("id").and_then(Value::as_str).ok_or_else(|| {
            initialization_failure("Grok initialize response has an auth method without an id")
        })?;
        if id.is_empty() || id.len() > 256 || auth_methods.iter().any(|known| known == id) {
            return Err(initialization_failure(
                "Grok initialize response has an invalid or duplicate auth method id",
            ));
        }
        auth_methods.push(id.to_owned());
    }
    let agent_info = result.get("agentInfo");
    let agent_name = agent_info
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("grok-build")
        .to_owned();
    let agent_version = agent_info
        .and_then(|info| info.get("version"))
        .and_then(Value::as_str)
        .unwrap_or("acp-v1")
        .to_owned();
    if agent_name.is_empty()
        || agent_name.len() > 256
        || agent_version.is_empty()
        || agent_version.len() > 256
    {
        return Err(initialization_failure(
            "Grok initialize response has invalid agentInfo",
        ));
    }
    let load_session = result
        .pointer("/agentCapabilities/loadSession")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let model_state = result.pointer("/_meta/modelState");
    let current_model_id = model_state
        .and_then(|state| state.get("currentModelId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut available_models = Vec::new();
    if let Some(models) = model_state
        .and_then(|state| state.get("availableModels"))
        .and_then(Value::as_array)
    {
        for model in models {
            let id = model
                .get("modelId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    initialization_failure("Grok modelState contains a model without `modelId`")
                })?;
            let label = model.get("name").and_then(Value::as_str).unwrap_or(id);
            if id.is_empty()
                || id.len() > 256
                || label.is_empty()
                || label.len() > 256
                || id.chars().any(char::is_control)
                || label.chars().any(char::is_control)
                || available_models.iter().any(|(known, _)| known == id)
            {
                return Err(initialization_failure(
                    "Grok modelState contains an invalid or duplicate model",
                ));
            }
            available_models.push((id.to_owned(), label.to_owned()));
        }
    }
    if current_model_id
        .as_ref()
        .is_some_and(|current| !available_models.iter().any(|(model, _)| model == current))
    {
        return Err(initialization_failure(
            "Grok currentModelId is absent from availableModels",
        ));
    }
    Ok(InitializeResult {
        agent_name,
        agent_version,
        auth_methods,
        load_session,
        current_model_id,
        available_models,
    })
}

pub(super) fn decode_account_identity(
    authentication: &Value,
) -> Result<(String, Vec<(String, String)>), BackendFailure> {
    decode_optional_account_identity(authentication).ok_or_else(|| {
        protocol_failure("Grok authenticate response has no stable account id or verified email")
    })
}

fn decode_optional_account_identity(
    authentication: &Value,
) -> Option<(String, Vec<(String, String)>)> {
    let metadata = authentication.get("_meta").and_then(Value::as_object);
    let email = metadata
        .and_then(|metadata| metadata.get("email"))
        .and_then(Value::as_str)
        .filter(|value| valid_account_text(value));
    let tier = metadata
        .and_then(|metadata| metadata.get("subscription_tier"))
        .and_then(Value::as_str)
        .filter(|value| valid_account_text(value));
    let (label, evidence) = if let Some(email) = email {
        (
            email.to_owned(),
            vec![("email".to_owned(), email.to_owned())],
        )
    } else if let Some(tier) = tier {
        (
            tier.to_owned(),
            vec![("subscription_tier".to_owned(), tier.to_owned())],
        )
    } else {
        (
            "local".to_owned(),
            vec![("local".to_owned(), "local".to_owned())],
        )
    };
    Some((label, evidence))
}

pub(super) fn decode_account_capacity_identity(
    authentication: &Value,
) -> Result<(String, Vec<(String, String)>), BackendFailure> {
    let (label, evidence) = decode_account_identity(authentication)?;
    if matches!(evidence.first().map(|(key, _)| key.as_str()), Some("email")) {
        return Ok((label, evidence));
    }
    Err(protocol_failure(
        "Grok authenticate response has no valid `email`",
    ))
}

fn valid_account_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

pub(super) fn decode_account_capacity(
    authentication: Value,
    primary: Option<AccountCapacityWindow>,
    account: AccountId,
) -> Result<AccountCapacitySnapshot, BackendFailure> {
    let metadata = authentication
        .get("_meta")
        .and_then(Value::as_object)
        .ok_or_else(|| protocol_failure("Grok authenticate response has no `_meta` object"))?;
    let subscription_tier = metadata
        .get("subscription_tier")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            protocol_failure("Grok authenticate response has no string `subscription_tier`")
        })?;
    if subscription_tier.is_empty()
        || subscription_tier.len() > 256
        || subscription_tier.trim() != subscription_tier
        || subscription_tier.chars().any(char::is_control)
    {
        return Err(protocol_failure(
            "Grok authenticate response has an invalid `subscription_tier`",
        ));
    }
    let provider = ProviderId::new("grok").map_err(|error| protocol_failure(error.to_string()))?;
    Ok(AccountCapacitySnapshot::new(
        provider,
        account,
        vec![AccountCapacityBucket::new(
            Some("grok".to_owned()),
            None,
            Some(subscription_tier.to_owned()),
            primary,
            None,
            None,
            None,
        )],
    ))
}

pub(super) fn string_at<'a>(value: &'a Value, path: &[&str]) -> Result<&'a str, BackendFailure> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or_else(|| {
            protocol_failure(format!("Grok ACP message is missing `{}`", path.join(".")))
        })?;
    }
    current.as_str().ok_or_else(|| {
        protocol_failure(format!(
            "Grok ACP field `{}` is not a string",
            path.join(".")
        ))
    })
}

pub(super) fn protocol_failure(message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(BackendFailureKind::Protocol, message)
}

fn initialization_failure(message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(BackendFailureKind::Initialization, message)
}

#[cfg(test)]
mod tests;
