use std::{collections::BTreeMap, fmt};

use serde::Deserialize;
use serde_json::{Value, json};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
    AccountId, BackendFailure, BackendFailureKind, ProviderId,
};

const SUPPORTED_CODEX_MAJOR: u64 = 0;
const SUPPORTED_CODEX_MINORS: &[u64] = &[145, 146, 149];
const MAX_USER_AGENT_DISPLAY_BYTES: usize = 256;

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
    #[serde(skip)]
    pub compatibility_warning: Option<CodexCompatibilityWarning>,
}

/// A bounded, terminal-safe compatibility warning from the Codex app-server handshake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexCompatibilityWarning {
    display_user_agent: String,
}

impl fmt::Display for CodexCompatibilityWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let supported = SUPPORTED_CODEX_MINORS
            .iter()
            .map(|minor| format!("{SUPPORTED_CODEX_MAJOR}.{minor}"))
            .collect::<Vec<_>>()
            .join(", ");
        formatter.write_str("Codex app-server `")?;
        formatter.write_str(&self.display_user_agent)?;
        write!(
            formatter,
            "` is newer or otherwise unverified; continuing because its 0.x protocol major matches (verified minor lines: {supported})"
        )
    }
}

#[derive(Debug)]
pub(super) struct ModelListPage {
    pub(super) models: Vec<(String, String, bool)>,
    pub(super) next_cursor: Option<String>,
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
    let mut initialize = initialize;
    initialize.compatibility_warning = version_compatibility_warning(&initialize.user_agent)?;
    if initialize.platform_family != "unix"
        || !matches!(initialize.platform_os.as_str(), "linux" | "macos")
    {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            "unsupported Codex app-server platform; expected unix/linux or unix/macos",
        ));
    }
    Ok(initialize)
}

pub(super) fn decode_account_capacity(
    result: Value,
    account: AccountId,
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
    Ok(AccountCapacitySnapshot::new(provider, account, buckets))
}

pub(super) fn decode_account_identity(
    result: &Value,
) -> Result<(String, Vec<(String, String)>), BackendFailure> {
    let (email, plan, stable_id) = account_identity_fields(result);
    let label = email.or(plan).unwrap_or("local").to_owned();
    let evidence = match (stable_id, email, plan) {
        (Some(stable_id), _, _) => vec![("account_id".to_owned(), stable_id.to_owned())],
        (None, Some(email), _) => vec![("email".to_owned(), email.to_owned())],
        (None, None, Some(plan)) => vec![("subscription".to_owned(), plan.to_owned())],
        (None, None, None) => vec![("local".to_owned(), "local".to_owned())],
    };
    Ok((label, evidence))
}

pub(super) fn decode_optional_account_identity(
    result: &Value,
) -> Option<(String, Vec<(String, String)>)> {
    let (email, plan, stable_id) = account_identity_fields(result);
    let label = email.or(plan).unwrap_or("local").to_owned();
    match (stable_id, email) {
        (Some(stable_id), _) => {
            Some((label, vec![("account_id".to_owned(), stable_id.to_owned())]))
        },
        (None, Some(email)) => Some((label, vec![("email".to_owned(), email.to_owned())])),
        (None, None) => None,
    }
}

fn account_identity_fields(result: &Value) -> (Option<&str>, Option<&str>, Option<&str>) {
    let account = result.get("account").unwrap_or(result);
    let email = account
        .get("email")
        .and_then(Value::as_str)
        .filter(|value| valid_catalog_text(value));
    let plan = account
        .get("planType")
        .and_then(Value::as_str)
        .filter(|value| valid_catalog_text(value));
    let stable_id = account
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| valid_catalog_text(value));
    (email, plan, stable_id)
}

pub(super) fn decode_account_capacity_identity(
    result: &Value,
) -> Result<(String, Vec<(String, String)>), BackendFailure> {
    let account = result.get("account").unwrap_or(result);
    let email = account
        .get("email")
        .and_then(Value::as_str)
        .filter(|value| valid_catalog_text(value))
        .filter(|value| AccountId::new((*value).to_owned()).is_ok())
        .ok_or_else(|| protocol_failure("Codex account/read response has no valid `email`"))?;
    let stable_id = account
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| valid_catalog_text(value));
    let evidence = stable_id.map_or_else(
        || vec![("email".to_owned(), email.to_owned())],
        |stable_id| vec![("account_id".to_owned(), stable_id.to_owned())],
    );
    Ok((email.to_owned(), evidence))
}

pub(super) fn decode_model_list(result: Value) -> Result<ModelListPage, BackendFailure> {
    let data = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol_failure("invalid Codex model/list response: missing `data`"))?;
    let mut models = Vec::new();
    for entry in data {
        if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let id = entry.get("model").and_then(Value::as_str).ok_or_else(|| {
            protocol_failure("invalid Codex model/list response: model has no `model`")
        })?;
        let label = entry
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(id);
        if !valid_catalog_text(id) || !valid_catalog_text(label) {
            return Err(protocol_failure(
                "invalid Codex model/list response: invalid model id or display name",
            ));
        }
        models.push((
            id.to_owned(),
            label.to_owned(),
            entry
                .get("isDefault")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ));
    }
    let next_cursor = match result.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) if valid_catalog_text(cursor) => Some(cursor.clone()),
        _ => {
            return Err(protocol_failure(
                "invalid Codex model/list response: invalid nextCursor",
            ));
        },
    };
    Ok(ModelListPage {
        models,
        next_cursor,
    })
}

fn valid_catalog_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value.trim() == value
        && !value.chars().any(char::is_control)
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

fn version_compatibility_warning(
    user_agent: &str,
) -> Result<Option<CodexCompatibilityWarning>, BackendFailure> {
    let display_user_agent = safe_user_agent(user_agent);
    let version = user_agent
        .split_whitespace()
        .find_map(|part| part.split_once('/').map(|(_, version)| version))
        .unwrap_or(user_agent);
    let mut components = version.split('.');
    let major = components.next().and_then(|part| part.parse::<u64>().ok());
    let minor = components.next().and_then(|part| part.parse::<u64>().ok());
    let Some(major) = major else {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            format!("Codex app-server returned an unparseable version in `{display_user_agent}`"),
        ));
    };
    let Some(minor) = minor else {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            format!("Codex app-server returned an unparseable version in `{display_user_agent}`"),
        ));
    };
    if major != SUPPORTED_CODEX_MAJOR {
        return Err(BackendFailure::new(
            BackendFailureKind::Initialization,
            format!(
                "unsupported Codex app-server major version in `{display_user_agent}`; yo requires {SUPPORTED_CODEX_MAJOR}.x"
            ),
        ));
    }
    if SUPPORTED_CODEX_MINORS.contains(&minor) {
        return Ok(None);
    }
    Ok(Some(CodexCompatibilityWarning { display_user_agent }))
}

fn safe_user_agent(user_agent: &str) -> String {
    const ELLIPSIS: &str = "…";
    let mut output = String::new();
    for character in user_agent.chars() {
        let rendered = if character.is_control() {
            character.escape_default().collect::<String>()
        } else {
            character.to_string()
        };
        if output.len() + rendered.len() + ELLIPSIS.len() > MAX_USER_AGENT_DISPLAY_BYTES {
            output.push_str(ELLIPSIS);
            break;
        }
        output.push_str(&rendered);
    }
    output
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
    use serde_json::json;

    use super::*;

    // 실제 wire 흐름을 검증한 Codex 0.145, 0.146, 0.149 minor line은 실행 파일 이름과
    // userAgent의 부가 문자열이 달라도 허용하고, 각 patch 업데이트는 다시 막지 않는다.
    #[test]
    fn accepts_each_verified_codex_minor_line() {
        assert_eq!(
            version_compatibility_warning("codex_cli_rs/0.145.3 (Linux)").unwrap(),
            None
        );
        assert_eq!(
            version_compatibility_warning("yo/0.146.0 (Arch Linux; x86_64)").unwrap(),
            None
        );
        assert_eq!(
            version_compatibility_warning("codex_cli_rs/0.149.0 (Linux)").unwrap(),
            None
        );
    }

    // 같은 protocol major의 새 minor line은 설치 업데이트만으로 Yo가 막히지 않게 허용하되,
    // 경고에 실제 userAgent와 검증된 line을 함께 남겨 호환성 불확실성을 숨기지 않는다.
    #[test]
    fn warns_for_an_unverified_codex_minor_line() {
        let warning = version_compatibility_warning("codex_cli_rs/0.150.0")
            .unwrap()
            .expect("an unverified minor line must produce a warning");

        let warning = warning.to_string();
        assert!(warning.contains("codex_cli_rs/0.150.0"));
        assert!(warning.contains("0.145, 0.146, 0.149"));
    }

    // protocol major가 달라지거나 version을 해석할 수 없으면 minor 업데이트와 구분해
    // 초기화 전에 계속 거부하여 실제 비호환 wire를 무조건 실행하지 않는다.
    #[test]
    fn rejects_a_different_or_unparseable_protocol_major() {
        let failure = version_compatibility_warning("codex_cli_rs/1.0.0").unwrap_err();
        assert_eq!(failure.kind(), BackendFailureKind::Initialization);
        assert!(failure.message().contains("requires 0.x"));

        let failure = version_compatibility_warning("codex_cli_rs/unknown").unwrap_err();
        assert_eq!(failure.kind(), BackendFailureKind::Initialization);
        assert!(failure.message().contains("unparseable version"));
    }

    // warning과 version failure가 제어문자와 과도한 길이를 포함해도 bounded 한 줄 안전 출력인지
    // 확인합니다.
    #[test]
    fn bounds_and_escapes_user_agent_in_warning_and_version_errors() {
        let user_agent = format!("codex_cli_rs/0.150.0\n\u{1b}[31m{}", "x".repeat(512));
        let display_user_agent = safe_user_agent(&user_agent);
        assert!(display_user_agent.len() <= MAX_USER_AGENT_DISPLAY_BYTES);
        assert!(!display_user_agent.contains('\n'));
        assert!(!display_user_agent.contains('\u{1b}'));

        let warning = version_compatibility_warning(&user_agent)
            .unwrap()
            .expect("an unverified minor line must produce a warning");

        let warning = warning.to_string();
        assert!(warning.len() <= 512);
        assert!(!warning.contains('\n'));
        assert!(!warning.contains('\u{1b}'));
        assert!(warning.contains("\\n"));
        assert!(warning.contains("\\u{1b}"));

        let malformed = format!("codex_cli_rs/unknown\n\u{1b}[31m{}", "x".repeat(512));
        let failure =
            version_compatibility_warning(&malformed).expect_err("the malformed version must fail");
        assert!(failure.message().len() <= 512);
        assert!(!failure.message().contains('\n'));
        assert!(!failure.message().contains('\u{1b}'));
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

    // model/list는 host가 숨긴 항목을 다시 노출하지 않고 exact model ID, display label,
    // default marker와 pagination cursor를 함께 보존합니다.
    #[test]
    fn decodes_only_visible_codex_models_with_default_and_cursor() {
        let page = decode_model_list(json!({
            "data": [
                {"id": "one", "model": "gpt-5.6-codex", "displayName": "GPT-5.6 Codex", "hidden": false, "isDefault": true},
                {"id": "hidden", "model": "internal", "displayName": "Internal", "hidden": true}
            ],
            "nextCursor": "page-2"
        }))
        .unwrap();

        assert_eq!(
            page.models,
            vec![("gpt-5.6-codex".to_owned(), "GPT-5.6 Codex".to_owned(), true)]
        );
        assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
    }

    // account section은 verified email을 사람이 보는 label로 사용하고 native id가 있으면
    // 기존 stable AccountId fingerprint를 보존하며, 없을 때만 email을 증거로 사용합니다.
    #[test]
    fn codex_account_identity_uses_email_label_and_evidence() {
        let (label, evidence) = decode_account_identity(&json!({
            "account": {"type": "chatgpt", "id": "acct-1", "email": "person@example.test", "planType": "pro"}
        }))
        .unwrap();

        assert_eq!(label, "person@example.test");
        assert_eq!(
            evidence,
            vec![("account_id".to_owned(), "acct-1".to_owned())]
        );
    }

    // email, subscription, local 순서의 catalog label fallback은 host inventory를 계속
    // 표시하고, native id가 없을 때도 선택한 evidence로 안정적인 계정을 만듭니다.
    #[test]
    fn codex_account_identity_falls_back_to_subscription_or_local() {
        let (label, evidence) = decode_account_identity(&json!({
            "account": {"type": "chatgpt", "planType": "pro"}
        }))
        .unwrap();
        assert_eq!(label, "pro");
        assert_eq!(evidence[0].0, "subscription");

        let (label, evidence) = decode_account_identity(&json!({
            "account": {"type": "apiKey"}
        }))
        .unwrap();
        assert_eq!(label, "local");
        assert_eq!(evidence[0].0, "local");
    }

    // 이메일이 없는 Codex capacity identity는 공용 default로 추정하지 않고 거부합니다.
    #[test]
    fn codex_account_capacity_identity_requires_email() {
        let failure = decode_account_capacity_identity(&json!({
            "account": {"id": "acct-1", "planType": "pro"}
        }))
        .unwrap_err();

        assert!(failure.message().contains("no valid `email`"));
    }

    // native id가 없어도 검증된 이메일을 Codex capacity identity로 사용할 수 있습니다.
    #[test]
    fn codex_account_capacity_identity_uses_email_when_native_id_is_absent() {
        let (label, evidence) = decode_account_capacity_identity(&json!({
            "account": {"email": "person@example.test", "planType": "pro"}
        }))
        .unwrap();

        assert_eq!(label, "person@example.test");
        assert_eq!(
            evidence,
            vec![("email".to_owned(), "person@example.test".to_owned())]
        );
    }

    // cache가 허용하는 AccountId 경계 안의 이메일만 capacity identity로 통과시킵니다.
    #[test]
    fn codex_account_capacity_identity_rejects_an_email_that_cannot_be_cached() {
        let accepted = "a".repeat(256);
        let (label, _) = decode_account_capacity_identity(&json!({
            "account": {"email": accepted}
        }))
        .unwrap();
        assert_eq!(label.len(), 256);

        let rejected = "a".repeat(257);
        let failure = decode_account_capacity_identity(&json!({
            "account": {"email": rejected}
        }))
        .unwrap_err();
        assert!(failure.message().contains("no valid `email`"));
    }
}
