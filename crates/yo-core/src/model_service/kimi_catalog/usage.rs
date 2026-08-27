use std::{error::Error, fmt, time::Duration};

use futures_util::StreamExt;
use jiff::Timestamp;
use reqwest::{Client, Url, header, redirect};
use serde_json::{Map, Value};
use tokio::time::{Instant, timeout_at};

use super::{KimiCatalogProduct, KimiCatalogSeed};
use crate::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
    ApiCredential,
};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_LIMIT_ROWS: usize = 32;
const MAX_NAME_BYTES: usize = 256;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const MINUTES_PER_WEEK: u64 = 7 * 24 * 60;
const FIXED_POINT_PER_CENT: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KimiAccountCapacityFailureKind {
    Configuration,
    Transport,
    HttpStatus,
    MediaType,
    Limit,
    Protocol,
    Timeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KimiAccountCapacityError {
    kind: KimiAccountCapacityFailureKind,
    message: String,
}

impl KimiAccountCapacityError {
    fn new(kind: KimiAccountCapacityFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> KimiAccountCapacityFailureKind {
        self.kind
    }
}

impl fmt::Display for KimiAccountCapacityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl Error for KimiAccountCapacityError {}

pub fn read_kimi_account_capacity(
    seed: &KimiCatalogSeed,
    credential: &ApiCredential,
) -> Result<AccountCapacitySnapshot, KimiAccountCapacityError> {
    require_code_membership(seed)?;
    let profile_url = seed.endpoint.append_path_segment("me").map_err(|_| {
        failure(
            KimiAccountCapacityFailureKind::Configuration,
            "Kimi Code endpoint cannot accept the account-profile path",
        )
    })?;
    let usage_url = seed.endpoint.append_path_segment("usages").map_err(|_| {
        failure(
            KimiAccountCapacityFailureKind::Configuration,
            "Kimi Code endpoint cannot accept the usages path",
        )
    })?;
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| {
            failure(
                KimiAccountCapacityFailureKind::Configuration,
                "cannot initialize the Kimi account-capacity HTTP client",
            )
        })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| {
            failure(
                KimiAccountCapacityFailureKind::Transport,
                "cannot initialize the Kimi account-capacity request runtime",
            )
        })?;
    let profile_bytes = runtime.block_on(fetch(&client, profile_url, credential))?;
    let plan = parse_kimi_account_plan(&profile_bytes)?;
    let usage_bytes = runtime.block_on(fetch(&client, usage_url, credential))?;
    parse_kimi_account_capacity_snapshot_with_plan(seed, &usage_bytes, Some(plan))
}

pub fn parse_kimi_account_capacity_snapshot(
    seed: &KimiCatalogSeed,
    bytes: &[u8],
) -> Result<AccountCapacitySnapshot, KimiAccountCapacityError> {
    parse_kimi_account_capacity_snapshot_with_plan(seed, bytes, None)
}

fn parse_kimi_account_capacity_snapshot_with_plan(
    seed: &KimiCatalogSeed,
    bytes: &[u8],
    plan: Option<String>,
) -> Result<AccountCapacitySnapshot, KimiAccountCapacityError> {
    require_code_membership(seed)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(limit_failure(
            "Kimi account-capacity response exceeds 1 MiB",
        ));
    }
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|_| protocol_failure("Kimi account-capacity response is not valid JSON"))?;
    let object = root
        .as_object()
        .ok_or_else(|| protocol_failure("Kimi account-capacity response must be an object"))?;

    let summary = object
        .get("usage")
        .filter(|value| !value.is_null())
        .map(|value| decode_usage_row(value, None, Some(MINUTES_PER_WEEK)))
        .transpose()?;
    let mut limits = decode_limit_rows(object)?;
    let credits = decode_booster_credits(object.get("boosterWallet"))?;

    let mut primary = summary;
    if primary.is_none() && !limits.is_empty() {
        primary = Some(limits.remove(0));
    }
    let secondary = if limits.is_empty() {
        None
    } else {
        Some(limits.remove(0))
    };
    if primary.is_none() && secondary.is_none() && limits.is_empty() && credits.is_none() {
        return Err(protocol_failure(
            "Kimi account-capacity response contains no usable capacity data",
        ));
    }

    let mut buckets = Vec::with_capacity(1 + limits.len());
    let main_reason = exhausted_reason(primary.as_ref(), secondary.as_ref());
    buckets.push(AccountCapacityBucket::new(
        Some("kimi".to_owned()),
        None,
        plan.clone(),
        primary.map(|row| row.window),
        secondary.map(|row| row.window),
        credits,
        main_reason,
    ));
    for (index, row) in limits.into_iter().enumerate() {
        let reason = row.exhausted.then(|| "usage_limit_reached".to_owned());
        buckets.push(AccountCapacityBucket::new(
            Some(format!("kimi-limit-{}", index + 2)),
            row.name,
            plan.clone(),
            Some(row.window),
            None,
            None,
            reason,
        ));
    }

    Ok(AccountCapacitySnapshot::new(
        seed.provider.clone(),
        seed.account.clone(),
        buckets,
    ))
}

fn parse_kimi_account_plan(bytes: &[u8]) -> Result<String, KimiAccountCapacityError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(limit_failure("Kimi account-profile response exceeds 1 MiB"));
    }
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|_| protocol_failure("Kimi account-profile response is not valid JSON"))?;
    let object = root
        .as_object()
        .ok_or_else(|| protocol_failure("Kimi account-profile response must be an object"))?;
    optional_name(object.get("user_level_name"))?
        .ok_or_else(|| protocol_failure("Kimi account-profile response has no user_level_name"))
}

fn require_code_membership(seed: &KimiCatalogSeed) -> Result<(), KimiAccountCapacityError> {
    if seed.product == KimiCatalogProduct::CodeMembership {
        Ok(())
    } else {
        Err(failure(
            KimiAccountCapacityFailureKind::Configuration,
            "account capacity is supported only for a stored Kimi Code Membership account",
        ))
    }
}

struct DecodedUsageRow {
    name: Option<String>,
    window: AccountCapacityWindow,
    exhausted: bool,
}

fn decode_limit_rows(
    object: &Map<String, Value>,
) -> Result<Vec<DecodedUsageRow>, KimiAccountCapacityError> {
    let Some(raw_limits) = object.get("limits") else {
        return Ok(Vec::new());
    };
    if raw_limits.is_null() {
        return Ok(Vec::new());
    }
    let raw_limits = raw_limits
        .as_array()
        .ok_or_else(|| protocol_failure("Kimi account-capacity limits must be an array"))?;
    if raw_limits.len() > MAX_LIMIT_ROWS {
        return Err(limit_failure(
            "Kimi account-capacity response exceeds 32 limit rows",
        ));
    }
    raw_limits
        .iter()
        .map(|value| {
            let item = value.as_object().ok_or_else(|| {
                protocol_failure("Kimi account-capacity limit row must be an object")
            })?;
            let detail = item
                .get("detail")
                .ok_or_else(|| protocol_failure("Kimi account-capacity limit row has no detail"))?;
            let name = optional_name(item.get("name"))?;
            let minutes = decode_window_minutes(item.get("window"))?;
            decode_usage_row(detail, name, Some(minutes))
        })
        .collect()
}

fn decode_usage_row(
    value: &Value,
    fallback_name: Option<String>,
    duration_minutes: Option<u64>,
) -> Result<DecodedUsageRow, KimiAccountCapacityError> {
    let object = value
        .as_object()
        .ok_or_else(|| protocol_failure("Kimi account-capacity usage row must be an object"))?;
    let used = optional_u64(object, "used")?.unwrap_or(0);
    let limit = optional_u64(object, "limit")?.unwrap_or(0);
    if limit == 0 {
        return Err(protocol_failure(
            "Kimi account-capacity usage limit must be positive",
        ));
    }
    let resets_at = optional_reset_time(object.get("resetTime"))?;
    let window = AccountCapacityWindow::from_usage_ratio(used, limit, duration_minutes, resets_at)
        .map_err(|error| protocol_failure(error.to_string()))?;
    Ok(DecodedUsageRow {
        name: optional_name(object.get("name"))?.or(fallback_name),
        window,
        exhausted: used >= limit,
    })
}

fn decode_window_minutes(value: Option<&Value>) -> Result<u64, KimiAccountCapacityError> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| protocol_failure("Kimi account-capacity limit has no valid window"))?;
    let duration = required_u64(object, "duration")?;
    if duration == 0 {
        return Err(protocol_failure(
            "Kimi account-capacity window duration must be positive",
        ));
    }
    let factor = match object.get("timeUnit").and_then(Value::as_str) {
        Some("TIME_UNIT_MINUTE") => 1,
        Some("TIME_UNIT_HOUR") => 60,
        Some("TIME_UNIT_DAY") => 24 * 60,
        Some("TIME_UNIT_WEEK") => MINUTES_PER_WEEK,
        _ => {
            return Err(protocol_failure(
                "Kimi account-capacity window has an unsupported time unit",
            ));
        },
    };
    duration
        .checked_mul(factor)
        .ok_or_else(|| protocol_failure("Kimi account-capacity window duration overflowed"))
}

fn optional_reset_time(value: Option<&Value>) -> Result<Option<i64>, KimiAccountCapacityError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let raw = value
        .as_str()
        .ok_or_else(|| protocol_failure("Kimi account-capacity resetTime must be a string"))?;
    let timestamp = raw
        .parse::<Timestamp>()
        .map_err(|_| protocol_failure("Kimi account-capacity resetTime is not RFC 3339"))?;
    Ok(Some(timestamp.as_second()))
}

fn optional_name(value: Option<&Value>) -> Result<Option<String>, KimiAccountCapacityError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let name = value
        .as_str()
        .ok_or_else(|| protocol_failure("Kimi account-capacity name must be a string"))?;
    if name.is_empty() || name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(protocol_failure(
            "Kimi account-capacity name is outside the bounded text profile",
        ));
    }
    Ok(Some(name.to_owned()))
}

fn required_u64(object: &Map<String, Value>, field: &str) -> Result<u64, KimiAccountCapacityError> {
    let value = object
        .get(field)
        .ok_or_else(|| protocol_failure(format!("Kimi account-capacity row has no {field}")))?;
    if let Some(value) = value.as_u64() {
        return Ok(value);
    }
    value
        .as_str()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            protocol_failure(format!(
                "Kimi account-capacity {field} must be a non-negative integer"
            ))
        })
}

fn optional_u64(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Option<u64>, KimiAccountCapacityError> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if let Some(value) = value.as_u64() {
        return Ok(Some(value));
    }
    value
        .as_str()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Some)
        .ok_or_else(|| {
            protocol_failure(format!(
                "Kimi account-capacity {field} must be a non-negative integer"
            ))
        })
}

fn decode_booster_credits(
    value: Option<&Value>,
) -> Result<Option<AccountCredits>, KimiAccountCapacityError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let wallet = value
        .as_object()
        .ok_or_else(|| protocol_failure("Kimi account-capacity boosterWallet must be an object"))?;
    let Some(balance) = wallet.get("balance").and_then(Value::as_object) else {
        return Ok(None);
    };
    if balance.get("type").and_then(Value::as_str) != Some("BOOSTER") {
        return Ok(None);
    }
    let amount = required_u64(balance, "amount")?;
    if amount == 0 {
        return Ok(None);
    }
    let amount_left = balance
        .get("amountLeft")
        .map(|_| required_u64(balance, "amountLeft"))
        .transpose()?
        .unwrap_or(0);
    let balance_cents = fixed_point_to_cents(amount_left);
    let currency = wallet
        .get("monthlyChargeLimit")
        .and_then(Value::as_object)
        .and_then(|value| value.get("currency"))
        .or_else(|| {
            wallet
                .get("monthlyUsed")
                .and_then(Value::as_object)
                .and_then(|value| value.get("currency"))
        })
        .and_then(Value::as_str)
        .unwrap_or("USD");
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(protocol_failure(
            "Kimi account-capacity currency is not a three-letter code",
        ));
    }
    let formatted = format!(
        "{} {}.{:02}",
        currency.to_ascii_uppercase(),
        balance_cents / 100,
        balance_cents % 100
    );
    Ok(Some(AccountCredits::new(
        Some(formatted),
        balance_cents > 0,
        false,
    )))
}

fn fixed_point_to_cents(value: u64) -> u64 {
    if value == 0 {
        return 0;
    }
    value
        .saturating_add(FIXED_POINT_PER_CENT / 2)
        .checked_div(FIXED_POINT_PER_CENT)
        .unwrap_or(0)
        .max(1)
}

fn exhausted_reason(
    primary: Option<&DecodedUsageRow>,
    secondary: Option<&DecodedUsageRow>,
) -> Option<String> {
    primary
        .into_iter()
        .chain(secondary)
        .any(|row| row.exhausted)
        .then(|| "usage_limit_reached".to_owned())
}

async fn fetch(
    client: &Client,
    request_url: Url,
    credential: &ApiCredential,
) -> Result<Vec<u8>, KimiAccountCapacityError> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let response = timeout_at(
        deadline,
        client
            .get(request_url)
            .bearer_auth(credential.expose_secret())
            .header(header::ACCEPT, "application/json")
            .send(),
    )
    .await
    .map_err(|_| timeout_failure("Kimi account-capacity request deadline expired"))?
    .map_err(map_reqwest_error)?;
    if response.status().is_redirection() {
        return Err(failure(
            KimiAccountCapacityFailureKind::Transport,
            "Kimi account-capacity endpoint redirected",
        ));
    }
    if !response.status().is_success() {
        return Err(failure(
            KimiAccountCapacityFailureKind::HttpStatus,
            format!(
                "Kimi account-capacity endpoint returned HTTP status {}",
                response.status().as_u16()
            ),
        ));
    }
    if !response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_json_media_type)
    {
        return Err(failure(
            KimiAccountCapacityFailureKind::MediaType,
            "Kimi account-capacity success did not return a JSON media type",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(limit_failure(
            "Kimi account-capacity response exceeds 1 MiB",
        ));
    }

    let mut bytes = Vec::new();
    let mut chunks = response.bytes_stream();
    let mut body_progress = Instant::now();
    loop {
        let body_deadline = deadline.min(body_progress + BODY_IDLE_TIMEOUT);
        let next = timeout_at(body_deadline, chunks.next())
            .await
            .map_err(|_| timeout_failure("Kimi account-capacity response-body deadline expired"))?;
        match next {
            Some(Ok(chunk)) if chunk.is_empty() => {},
            Some(Ok(chunk)) => {
                body_progress = Instant::now();
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(limit_failure(
                        "Kimi account-capacity response exceeds 1 MiB",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            },
            Some(Err(_)) => {
                return Err(failure(
                    KimiAccountCapacityFailureKind::Transport,
                    "Kimi account-capacity response body could not be read",
                ));
            },
            None => return Ok(bytes),
        }
    }
}

fn is_json_media_type(value: &str) -> bool {
    let media_type = value.split(';').next().unwrap_or("").trim();
    media_type.eq_ignore_ascii_case("application/json")
        || media_type
            .to_ascii_lowercase()
            .strip_prefix("application/")
            .is_some_and(|subtype| subtype.ends_with("+json"))
}

fn map_reqwest_error(error: reqwest::Error) -> KimiAccountCapacityError {
    if error.is_timeout() {
        timeout_failure("Kimi account-capacity transport deadline expired")
    } else {
        failure(
            KimiAccountCapacityFailureKind::Transport,
            "Kimi account-capacity HTTP request failed",
        )
    }
}

fn failure(
    kind: KimiAccountCapacityFailureKind,
    message: impl Into<String>,
) -> KimiAccountCapacityError {
    KimiAccountCapacityError::new(kind, message)
}

fn protocol_failure(message: impl Into<String>) -> KimiAccountCapacityError {
    failure(KimiAccountCapacityFailureKind::Protocol, message)
}

fn limit_failure(message: impl Into<String>) -> KimiAccountCapacityError {
    failure(KimiAccountCapacityFailureKind::Limit, message)
}

fn timeout_failure(message: impl Into<String>) -> KimiAccountCapacityError {
    failure(KimiAccountCapacityFailureKind::Timeout, message)
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use reqwest::{Client, redirect};
    use serde_json::json;

    use super::*;
    use crate::{
        AccountId, NormalizedEndpoint, ProviderId, VersionedProfileId,
        model_connector::tests::local_tls::{LocalServerMode, LocalTlsServer, run_in_tls_child},
    };

    fn code_seed() -> KimiCatalogSeed {
        KimiCatalogSeed::resolve(
            VersionedProfileId::new("kimi-code-membership/v1").unwrap(),
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            None,
            None,
        )
        .unwrap()
    }

    fn platform_seed() -> KimiCatalogSeed {
        KimiCatalogSeed::resolve(
            VersionedProfileId::new("kimi-platform-ai/v1").unwrap(),
            ProviderId::new("kimi").unwrap(),
            AccountId::new("default").unwrap(),
            None,
            None,
        )
        .unwrap()
    }

    fn usage_payload() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "usage": {
                "used": 92,
                "limit": 100,
                "resetTime": "2026-08-30T04:05:00Z"
            },
            "limits": [{
                "name": "rolling",
                "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                "detail": {
                    "used": "7",
                    "limit": "100",
                    "resetTime": "2026-08-27T17:36:00Z"
                }
            }],
            "boosterWallet": {
                "balance": {
                    "type": "BOOSTER",
                    "amount": "1000000000",
                    "amountLeft": "500000000"
                },
                "monthlyChargeLimit": {"currency": "USD"}
            }
        }))
        .unwrap()
    }

    // Kimi Code의 weekly summary와 rolling limit을 공용 두 window로 투영하고,
    // count 비율은 남은 양을 과장하지 않도록 올림한 used percentage로 정규화합니다.
    #[test]
    fn decodes_weekly_rolling_and_booster_capacity() {
        let snapshot =
            parse_kimi_account_capacity_snapshot(&code_seed(), &usage_payload()).unwrap();
        assert_eq!(snapshot.provider().as_str(), "kimi");
        assert_eq!(snapshot.account().as_str(), "default");
        let bucket = &snapshot.buckets()[0];
        let weekly = bucket.primary().unwrap();
        let rolling = bucket.secondary().unwrap();
        assert_eq!(weekly.used_percent(), 92);
        assert_eq!(weekly.window_duration_minutes(), Some(MINUTES_PER_WEEK));
        assert_eq!(rolling.used_percent(), 7);
        assert_eq!(rolling.window_duration_minutes(), Some(300));
        assert_eq!(bucket.credits().unwrap().balance(), Some("USD 5.00"));
        assert_eq!(bucket.limit_reason(), None);
    }

    // Kimi Code의 `/me`가 보고한 사용자 등급명은 공개 플랜명과 같은 값을 가지므로
    // 별도 추정 없이 모든 계정 한도 bucket의 공용 plan 필드로 전달합니다.
    #[test]
    fn decodes_account_level_name_as_the_capacity_plan() {
        let plan = parse_kimi_account_plan(
            &serde_json::to_vec(&json!({
                "user_level": 20,
                "user_level_name": "Moderato"
            }))
            .unwrap(),
        )
        .unwrap();
        let snapshot = parse_kimi_account_capacity_snapshot_with_plan(
            &code_seed(),
            &usage_payload(),
            Some(plan),
        )
        .unwrap();

        assert_eq!(snapshot.buckets()[0].plan(), Some("Moderato"));
        assert!(
            snapshot
                .buckets()
                .iter()
                .all(|bucket| bucket.plan() == Some("Moderato"))
        );
    }

    // `/me` 성공 응답에 등급명이 없거나 표시 경계를 벗어난 값이 있으면 플랜을
    // 만들어내지 않고 protocol failure로 닫아 Unknown과 유효한 등급을 혼동하지 않습니다.
    #[test]
    fn rejects_missing_or_unsafe_account_level_names() {
        for payload in [
            json!({}),
            json!({"user_level_name": null}),
            json!({"user_level_name": ""}),
            json!({"user_level_name": "unsafe\nname"}),
        ] {
            let error =
                parse_kimi_account_plan(&serde_json::to_vec(&payload).unwrap()).unwrap_err();
            assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Protocol);
        }
    }

    // 성공 응답이어도 usable capacity가 없거나 배열·window가 계약 밖이면 조용히
    // Unknown으로 만들지 않고 protocol failure로 닫습니다.
    #[test]
    fn rejects_empty_and_malformed_success_snapshots() {
        for payload in [
            json!({}),
            json!({"limits": {}}),
            json!({"limits": [{"window": {"duration": 5, "timeUnit": "HOUR"}, "detail": {"used": 1, "limit": 2}}]}),
            json!({"usage": {"used": 1, "limit": 0}}),
        ] {
            let error = parse_kimi_account_capacity_snapshot(
                &code_seed(),
                &serde_json::to_vec(&payload).unwrap(),
            )
            .unwrap_err();
            assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Protocol);
        }
    }

    // 공식 Kimi Code parser처럼 limit이 있는 row에서 생략된 used는 아직 사용하지 않은
    // 0으로 해석하지만, 분모인 limit 누락은 유효한 잔여량을 만들 수 없으므로 거절합니다.
    #[test]
    fn accepts_omitted_used_but_not_omitted_limit() {
        let snapshot = parse_kimi_account_capacity_snapshot(
            &code_seed(),
            &serde_json::to_vec(&json!({"usage": {"limit": "100"}})).unwrap(),
        )
        .unwrap();
        assert_eq!(snapshot.buckets()[0].primary().unwrap().used_percent(), 0);

        let error = parse_kimi_account_capacity_snapshot(
            &code_seed(),
            &serde_json::to_vec(&json!({"usage": {"used": "1"}})).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Protocol);
    }

    // `/usages`는 Kimi Code Membership의 계약이므로 같은 ProviderId라도 Platform
    // API key 계정에는 요청 전 parser 경계에서 적용되지 않습니다.
    #[test]
    fn rejects_kimi_platform_accounts() {
        let error =
            parse_kimi_account_capacity_snapshot(&platform_seed(), &usage_payload()).unwrap_err();
        assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Configuration);
    }

    // direct parser와 transport가 같은 1 MiB 상한을 가져 큰 원격 JSON을 부분 결과로
    // 해석하거나 호출 경로에 따라 메모리 한도를 달리하지 않습니다.
    #[test]
    fn direct_parser_keeps_the_response_byte_limit() {
        let error =
            parse_kimi_account_capacity_snapshot(&code_seed(), &vec![b' '; MAX_RESPONSE_BYTES + 1])
                .unwrap_err();
        assert_eq!(error.kind(), KimiAccountCapacityFailureKind::Limit);
    }

    // local TLS listener로 exact GET `/v1/usages`, JSON Accept, 단 한 연결과 Bearer
    // credential 비노출을 함께 관찰합니다.
    #[test]
    fn fetches_one_authenticated_usage_snapshot_over_local_tls() {
        if run_in_tls_child(
            "model_service::kimi_catalog::usage::tests::fetches_one_authenticated_usage_snapshot_over_local_tls",
        ) {
            return;
        }
        let body = usage_payload();
        let server = LocalTlsServer::start(LocalServerMode::Success {
            body: body.clone(),
            content_type: "application/json; charset=utf-8".to_owned(),
        });
        let root = env::var_os("YO_MODEL_CONNECTOR_TEST_ROOT").unwrap();
        let roots = reqwest::Certificate::from_pem_bundle(&fs::read(root).unwrap()).unwrap();
        let client = Client::builder()
            .add_root_certificate(roots[0].clone())
            .redirect(redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .unwrap();
        let url = NormalizedEndpoint::parse(server.endpoint())
            .unwrap()
            .append_path_segment("usages")
            .unwrap();
        let received = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fetch(
                &client,
                url,
                &ApiCredential::new("sentinel-kimi-usage-key").unwrap(),
            ))
            .unwrap();
        assert_eq!(received, body);
        server.wait_for_response_sent();
        let requests = server.requests();
        assert_eq!(server.accepted_connections(), 1);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["method"], "GET");
        assert_eq!(requests[0]["path"], "/v1/usages");
        assert_eq!(requests[0]["headers"]["accept"], "application/json");
        assert!(requests[0].get("authorization").is_none());
        assert!(requests[0]["authorization_sha256"].is_string());
        assert!(
            !serde_json::to_string(&requests)
                .unwrap()
                .contains("sentinel-kimi-usage-key")
        );
    }

    // Kimi Code 계정 플랜 조회도 usage와 같은 no-redirect·no-retry Bearer 경계를
    // 사용하며 exact GET `/v1/me` 응답만 profile parser에 전달합니다.
    #[test]
    fn fetches_one_authenticated_account_profile_over_local_tls() {
        if run_in_tls_child(
            "model_service::kimi_catalog::usage::tests::fetches_one_authenticated_account_profile_over_local_tls",
        ) {
            return;
        }
        let body = serde_json::to_vec(&json!({"user_level_name": "Moderato"})).unwrap();
        let server = LocalTlsServer::start(LocalServerMode::Success {
            body: body.clone(),
            content_type: "application/json".to_owned(),
        });
        let root = env::var_os("YO_MODEL_CONNECTOR_TEST_ROOT").unwrap();
        let roots = reqwest::Certificate::from_pem_bundle(&fs::read(root).unwrap()).unwrap();
        let client = Client::builder()
            .add_root_certificate(roots[0].clone())
            .redirect(redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .unwrap();
        let url = NormalizedEndpoint::parse(server.endpoint())
            .unwrap()
            .append_path_segment("me")
            .unwrap();
        let received = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fetch(
                &client,
                url,
                &ApiCredential::new("sentinel-kimi-profile-key").unwrap(),
            ))
            .unwrap();
        assert_eq!(parse_kimi_account_plan(&received).unwrap(), "Moderato");
        server.wait_for_response_sent();
        let requests = server.requests();
        assert_eq!(server.accepted_connections(), 1);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["method"], "GET");
        assert_eq!(requests[0]["path"], "/v1/me");
        assert_eq!(requests[0]["headers"]["accept"], "application/json");
        assert!(requests[0].get("authorization").is_none());
        assert!(requests[0]["authorization_sha256"].is_string());
        assert!(
            !serde_json::to_string(&requests)
                .unwrap()
                .contains("sentinel-kimi-profile-key")
        );
    }
}
