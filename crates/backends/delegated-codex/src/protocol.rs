use std::{collections::BTreeMap, fmt};

use serde::Deserialize;
use serde_json::{Value, json};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
    AccountId, ActivityNotice, BackendFailure, BackendFailureKind, NoticeLevel, ProviderId,
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

/// A bounded, terminal-safe Codex compatibility or server warning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexWarning {
    display_user_agent: String,
    notice: Option<ActivityNotice>,
    thread_id: Option<String>,
}

/// Historical observer argument name retained for source compatibility.
pub type CodexCompatibilityWarning = CodexWarning;

impl CodexWarning {
    /// Structured, terminal-safe presentation independent of a Turn or tool activity.
    pub fn to_notice(&self) -> ActivityNotice {
        self.notice.clone().unwrap_or_else(|| ActivityNotice {
            title: "Codex compatibility warning".to_owned(),
            message: self.to_string(),
            level: NoticeLevel::Warning,
        })
    }

    pub(super) fn from_notification(method: &str, params: &Value) -> Option<Self> {
        let (title, mut message) = match method {
            "warning" => ("Codex warning", params.get("message")?.as_str()?.to_owned()),
            "guardianWarning" => {
                params.get("threadId")?.as_str()?;
                (
                    "Codex approval warning",
                    params.get("message")?.as_str()?.to_owned(),
                )
            },
            "deprecationNotice" => (
                "Codex deprecation notice",
                params.get("summary")?.as_str()?.to_owned(),
            ),
            "configWarning" => (
                "Codex configuration warning",
                params.get("summary")?.as_str()?.to_owned(),
            ),
            _ => return None,
        };
        for (field, label) in [("details", "Details"), ("path", "File")] {
            if let Some(value) = params.get(field).and_then(Value::as_str) {
                message.push_str(&format!("\n{label}: {value}"));
            }
        }
        if let Some(range) = params.get("range").filter(|value| !value.is_null()) {
            message.push_str(&format!("\nRange: {range}"));
        }
        let thread_id = match params.get("threadId") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => Some(id.clone()),
            _ => return None,
        };
        let mut safe = String::new();
        for character in message.chars() {
            let rendered = if character.is_control() && !matches!(character, '\n' | '\t') {
                character.escape_default().collect::<String>()
            } else {
                character.to_string()
            };
            if safe.len() + rendered.len() + 3 > 8192 {
                safe.push('…');
                break;
            }
            safe.push_str(&rendered);
        }
        Some(Self {
            display_user_agent: String::new(),
            notice: Some(ActivityNotice {
                title: title.to_owned(),
                message: safe,
                level: NoticeLevel::Warning,
            }),
            thread_id,
        })
    }

    pub(super) fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }
}

impl fmt::Display for CodexWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(notice) = &self.notice {
            return write!(formatter, "{}: {}", notice.title, notice.message);
        }
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
    pub(super) models: Vec<ModelListModel>,
    pub(super) next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModelListModel {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) hidden: bool,
    pub(super) is_default: bool,
    pub(super) image_modality: ModelImageModality,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModelImageModality {
    Missing,
    Invalid,
    Explicit { image: bool },
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
        let image_modality = match entry.get("inputModalities") {
            None => ModelImageModality::Missing,
            Some(Value::Array(values)) => {
                let mut text = false;
                let mut image = false;
                let mut valid = true;
                for value in values {
                    match value.as_str() {
                        Some("text") if !text => text = true,
                        Some("image") if !image => image = true,
                        _ => valid = false,
                    }
                }
                if valid {
                    ModelImageModality::Explicit { image }
                } else {
                    ModelImageModality::Invalid
                }
            },
            Some(_) => ModelImageModality::Invalid,
        };
        models.push(ModelListModel {
            id: id.to_owned(),
            label: label.to_owned(),
            hidden: entry
                .get("hidden")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            is_default: entry
                .get("isDefault")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            image_modality,
        });
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
    Ok(Some(CodexWarning {
        display_user_agent,
        notice: None,
        thread_id: None,
    }))
}

/// Returns true only for the exact reviewed image-capable Codex wire build.
pub(super) fn image_wire_version_supported(user_agent: &str) -> bool {
    user_agent
        .split_whitespace()
        .find_map(|part| part.split_once('/').map(|(_, version)| version))
        == Some("0.153.4")
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
mod tests;
