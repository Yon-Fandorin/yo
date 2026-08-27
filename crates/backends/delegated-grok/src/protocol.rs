use serde_json::{Value, json};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountId, BackendFailure, BackendFailureKind,
    ProviderId,
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
                protocol_failure("response id from Grok ACP must be an unsigned integer")
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
    Ok(InitializeResult {
        agent_name,
        agent_version,
        auth_methods,
        load_session,
    })
}

pub(super) fn decode_account_capacity(
    authentication: Value,
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
    let account = AccountId::new("default").map_err(|error| protocol_failure(error.to_string()))?;
    Ok(AccountCapacitySnapshot::new(
        provider,
        account,
        vec![AccountCapacityBucket::new(
            Some("grok".to_owned()),
            None,
            Some(subscription_tier.to_owned()),
            None,
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
mod tests {
    use serde_json::json;

    use super::*;

    // ACP 메시지는 JSON-RPC 2.0 표식을 생략하면 method와 id가 올바르더라도 wire 경계를
    // 통과하지 못해야 하며, 표식이 있는 server request의 문자열 id는 그대로 보존합니다.
    #[test]
    fn requires_json_rpc_two_and_preserves_server_request_ids() {
        assert!(classify(json!({ "method": "session/update", "params": {} })).is_err());
        assert!(
            classify(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {},
                "error": { "code": -32000, "message": "ambiguous" }
            }))
            .is_err()
        );
        let incoming = classify(json!({
            "jsonrpc": "2.0",
            "id": "permission-1",
            "method": "session/request_permission",
            "params": {}
        }))
        .unwrap();
        assert!(matches!(
            incoming,
            Incoming::ServerRequest { id, method, .. }
                if id == json!("permission-1") && method == "session/request_permission"
        ));

        for invalid_id in [json!(1.5), json!(true), json!([1]), json!({ "id": 1 })] {
            assert!(
                classify(json!({
                    "jsonrpc": "2.0",
                    "id": invalid_id,
                    "method": "session/request_permission",
                    "params": {}
                }))
                .is_err()
            );
        }
        assert!(
            classify(json!({
                "jsonrpc": "2.0",
                "id": "x".repeat(MAX_REQUEST_ID_BYTES + 1),
                "method": "session/request_permission",
                "params": {}
            }))
            .is_err()
        );
    }

    // 현재 backend가 구현하는 ACP v1만 초기화에서 수락하고, 다른 negotiated version은
    // Session을 만들기 전에 Initialization 실패로 구분합니다.
    #[test]
    fn accepts_only_acp_protocol_version_one() {
        let accepted =
            decode_initialize(json!({ "protocolVersion": 1, "authMethods": [] })).unwrap();
        assert_eq!(accepted.agent_version, "acp-v1");

        let failure =
            decode_initialize(json!({ "protocolVersion": 2, "authMethods": [] })).unwrap_err();
        assert_eq!(failure.kind(), BackendFailureKind::Initialization);
    }

    // 인증 method의 누락·잘못된 ID를 인증 불필요로 오인하면 cached login 경계를 우회하므로
    // authMethods는 배열과 각 고유 ID를 완전하게 검증합니다.
    #[test]
    fn rejects_malformed_or_duplicate_authentication_methods() {
        let missing = json!({
            "protocolVersion": 1,
            "agentCapabilities": {}
        });
        assert!(decode_initialize(missing).is_err());

        let malformed = json!({
            "protocolVersion": 1,
            "authMethods": [{ "name": "Cached" }]
        });
        assert!(decode_initialize(malformed).is_err());

        let duplicate = json!({
            "protocolVersion": 1,
            "authMethods": [
                { "id": "cached_token" },
                { "id": "cached_token" }
            ]
        });
        assert!(decode_initialize(duplicate).is_err());
    }
}
