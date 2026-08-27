use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
    AccountId, BackendFailure, BackendFailureKind, ProviderId,
};

const SUPPORTED_CODEX_MAJOR: u64 = 0;
const SUPPORTED_CODEX_MINORS: &[u64] = &[145, 146, 149];

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitializeResult {
    pub user_agent: String,
    pub platform_family: String,
    pub platform_os: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountRateLimitsResponse {
    rate_limits: WireRateLimitSnapshot,
    #[serde(default)]
    rate_limits_by_limit_id: Option<BTreeMap<String, WireRateLimitSnapshot>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireRateLimitSnapshot {
    #[serde(default)]
    credits: Option<WireCreditsSnapshot>,
    #[serde(default)]
    limit_id: Option<String>,
    #[serde(default)]
    limit_name: Option<String>,
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    primary: Option<WireRateLimitWindow>,
    #[serde(default)]
    rate_limit_reached_type: Option<String>,
    #[serde(default)]
    secondary: Option<WireRateLimitWindow>,
    #[serde(default)]
    spend_control_reached: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireRateLimitWindow {
    used_percent: i64,
    #[serde(default)]
    window_duration_mins: Option<i64>,
    #[serde(default)]
    resets_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireCreditsSnapshot {
    #[serde(default)]
    balance: Option<String>,
    has_credits: bool,
    unlimited: bool,
}

pub(super) fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "id": id, "method": method, "params": params })
}

pub(super) fn initialized_notification() -> Value {
    json!({ "method": "initialized" })
}

pub(super) fn server_response(id: Value, result: Value) -> Value {
    json!({ "id": id, "result": result })
}

pub(super) fn server_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "id": id, "error": { "code": code, "message": message } })
}

pub(super) fn classify(value: Value) -> Result<Incoming, BackendFailure> {
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

pub(super) fn decode_initialize(result: Value) -> Result<InitializeResult, BackendFailure> {
    let initialize: InitializeResult = serde_json::from_value(result).map_err(|error| {
        BackendFailure::new(
            BackendFailureKind::Initialization,
            format!("invalid Codex initialize response: {error}"),
        )
    })?;
    ensure_supported_version(&initialize.user_agent)?;
    if initialize.platform_family != "unix"
        || !matches!(initialize.platform_os.as_str(), "linux" | "macos")
    {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            format!(
                "unsupported Codex app-server platform {}/{}",
                initialize.platform_family, initialize.platform_os
            ),
        ));
    }
    Ok(initialize)
}

pub(super) fn decode_account_capacity(
    result: Value,
) -> Result<AccountCapacitySnapshot, BackendFailure> {
    let decoded: AccountRateLimitsResponse = serde_json::from_value(result).map_err(|error| {
        protocol_failure(format!(
            "invalid Codex account/rateLimits/read response: {error}"
        ))
    })?;
    let buckets = match decoded.rate_limits_by_limit_id {
        Some(buckets) if !buckets.is_empty() => buckets
            .into_iter()
            .map(|(id, bucket)| decode_capacity_bucket(Some(id), bucket))
            .collect::<Result<Vec<_>, _>>()?,
        _ => vec![decode_capacity_bucket(None, decoded.rate_limits)?],
    };
    let provider = ProviderId::new("codex").map_err(|error| protocol_failure(error.to_string()))?;
    let account = AccountId::new("default").map_err(|error| protocol_failure(error.to_string()))?;
    Ok(AccountCapacitySnapshot::new(provider, account, buckets))
}

fn decode_capacity_bucket(
    fallback_id: Option<String>,
    bucket: WireRateLimitSnapshot,
) -> Result<AccountCapacityBucket, BackendFailure> {
    let primary = bucket.primary.map(decode_capacity_window).transpose()?;
    let secondary = bucket.secondary.map(decode_capacity_window).transpose()?;
    let credits = bucket.credits.map(|credits| {
        AccountCredits::new(credits.balance, credits.has_credits, credits.unlimited)
    });
    let limit_reason = bucket.rate_limit_reached_type.or_else(|| {
        bucket
            .spend_control_reached
            .filter(|reached| *reached)
            .map(|_| "spend_control_reached".to_owned())
    });
    Ok(AccountCapacityBucket::new(
        bucket.limit_id.or(fallback_id),
        bucket.limit_name,
        bucket.plan_type,
        primary,
        secondary,
        credits,
        limit_reason,
    ))
}

fn decode_capacity_window(
    window: WireRateLimitWindow,
) -> Result<AccountCapacityWindow, BackendFailure> {
    let used_percent = u8::try_from(window.used_percent)
        .map_err(|_| protocol_failure("Codex account usedPercent is outside 0..=100"))?;
    let duration = window
        .window_duration_mins
        .map(u64::try_from)
        .transpose()
        .map_err(|_| protocol_failure("Codex account windowDurationMins is negative"))?;
    AccountCapacityWindow::new(used_percent, duration, window.resets_at)
        .map_err(|error| protocol_failure(error.to_string()))
}

fn ensure_supported_version(user_agent: &str) -> Result<(), BackendFailure> {
    let version = user_agent
        .split_whitespace()
        .find_map(|part| part.split_once('/').map(|(_, version)| version))
        .unwrap_or(user_agent);
    let mut components = version.split('.');
    let major = components.next().and_then(|part| part.parse::<u64>().ok());
    let minor = components.next().and_then(|part| part.parse::<u64>().ok());
    if major == Some(SUPPORTED_CODEX_MAJOR)
        && minor.is_some_and(|minor| SUPPORTED_CODEX_MINORS.contains(&minor))
    {
        return Ok(());
    }
    let supported = SUPPORTED_CODEX_MINORS
        .iter()
        .map(|minor| format!("{SUPPORTED_CODEX_MAJOR}.{minor}"))
        .collect::<Vec<_>>()
        .join(", ");
    Err(BackendFailure::new(
        BackendFailureKind::Initialization,
        format!(
            "unsupported Codex app-server version in `{user_agent}`; yo currently verifies {supported}"
        ),
    ))
}

pub(super) fn string_at<'a>(value: &'a Value, path: &[&str]) -> Result<&'a str, BackendFailure> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or_else(|| {
            protocol_failure(format!("Codex message is missing `{}`", path.join(".")))
        })?;
    }
    current.as_str().ok_or_else(|| {
        protocol_failure(format!("Codex field `{}` is not a string", path.join(".")))
    })
}

pub(super) fn protocol_failure(message: impl Into<String>) -> BackendFailure {
    BackendFailure::new(BackendFailureKind::Protocol, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 실제 wire 흐름을 검증한 Codex 0.145, 0.146, 0.149 minor line은 실행 파일 이름과
    // userAgent의 부가 문자열이 달라도 허용하고, 각 patch 업데이트는 다시 막지 않는다.
    #[test]
    fn accepts_each_verified_codex_minor_line() {
        assert!(ensure_supported_version("codex_cli_rs/0.145.3 (Linux)").is_ok());
        assert!(ensure_supported_version("yo/0.146.0 (Arch Linux; x86_64)").is_ok());
        assert!(ensure_supported_version("codex_cli_rs/0.149.0 (Linux)").is_ok());
    }

    // 아직 wire 호환성을 검증하지 않은 다음 Codex minor line은 시작 단계에서 명시적으로
    // 거절하고, 오류에는 현재 허용한 세 line을 모두 표시해 대응 범위를 알 수 있게 한다.
    #[test]
    fn rejects_an_unverified_codex_minor_line() {
        let failure = ensure_supported_version("codex_cli_rs/0.150.0").unwrap_err();
        assert_eq!(failure.kind(), BackendFailureKind::Initialization);
        assert!(failure.message().contains("0.145, 0.146, 0.149"));
    }

    // method와 id가 함께 있는 app-server 메시지는 일반 notification이 아니라 클라이언트가
    // 반드시 답해야 하는 server request로 보존되는지 확인한다.
    #[test]
    fn distinguishes_server_requests_from_notifications() {
        let incoming = classify(json!({
            "id": "approval-1",
            "method": "item/fileChange/requestApproval",
            "params": { "itemId": "item-1" }
        }))
        .unwrap();

        assert!(matches!(
            incoming,
            Incoming::ServerRequest { id, method, .. }
                if id == json!("approval-1") && method == "item/fileChange/requestApproval"
        ));
    }
}
