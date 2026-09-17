//! Codex JSON-RPC 메시지의 framing과 분류.
//!
//! JSON-RPC shape은 이 모듈에서만 구성하고, 런타임 이벤트 상태는 호출자에게
//! 남겨 둡니다.

use serde_json::{Value, json};
use yo_core::BackendFailure;

use super::bounds::protocol_failure;

#[derive(Debug)]
pub(crate) enum Incoming {
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

/// 요청 JSON-RPC object의 기존 필드 순서와 shape을 유지합니다.
pub(crate) fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "id": id, "method": method, "params": params })
}

/// initialize 뒤에 보내는 빈 initialized notification입니다.
pub(crate) fn initialized_notification() -> Value {
    json!({ "method": "initialized" })
}

/// 서버 요청에 대한 성공 응답 shape을 구성합니다.
pub(crate) fn server_response(id: Value, result: Value) -> Value {
    json!({ "id": id, "result": result })
}

/// 서버 요청에 대한 오류 응답 shape을 구성합니다.
pub(crate) fn server_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "id": id, "error": { "code": code, "message": message } })
}

/// 수신 JSON value를 response, notification, server request 중 하나로 분류합니다.
pub(crate) fn classify(value: Value) -> Result<Incoming, BackendFailure> {
    let object = value
        .as_object()
        .ok_or_else(|| protocol_failure("Codex app-server message must be a JSON object"))?;
    let method = object.get("method").and_then(Value::as_str);
    let id = object.get("id");

    match (method, id) {
        (Some(method), Some(id)) => Ok(Incoming::ServerRequest {
            id: id.clone(),
            method: method.to_owned(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(method), None) => Ok(Incoming::Notification {
            method: method.to_owned(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(id)) => {
            let id = id.as_u64().ok_or_else(|| {
                protocol_failure("response id from Codex app-server must be an unsigned integer")
            })?;
            if let Some(result) = object.get("result") {
                return Ok(Incoming::Response {
                    id,
                    result: result.clone(),
                });
            }
            let error = object
                .get("error")
                .and_then(Value::as_object)
                .ok_or_else(|| protocol_failure("response has neither result nor error"))?;
            let code = error.get("code").and_then(Value::as_i64).ok_or_else(|| {
                protocol_failure("Codex app-server error response has no numeric code")
            })?;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    protocol_failure("Codex app-server error response has no message")
                })?;
            Ok(Incoming::ResponseError {
                id,
                code,
                message: message.to_owned(),
            })
        },
        (None, None) => Err(protocol_failure(
            "Codex app-server message has neither method nor id",
        )),
    }
}
