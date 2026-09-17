//! Codex account identity, rate-limit, credit, and capacity decoding.
//!
//! 계정/용량 응답의 serde shape과 identity fallback을 보존하며 runtime state에는
//! 의존하지 않습니다.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
    AccountId, BackendFailure, ProviderId,
};

use super::bounds::{protocol_failure, valid_catalog_text};

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

/// Codex account/rateLimits response를 provider-neutral capacity snapshot으로 변환합니다.
pub(crate) fn decode_account_capacity(
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

/// 일반 model catalog용 account label과 안정적인 identity evidence를 추출합니다.
pub(crate) fn decode_account_identity(
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

/// binding 관찰용 account identity를 stable evidence가 있을 때만 반환합니다.
pub(crate) fn decode_optional_account_identity(
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

/// capacity 조회에서 cache 가능한 verified email identity를 요구합니다.
pub(crate) fn decode_account_capacity_identity(
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
