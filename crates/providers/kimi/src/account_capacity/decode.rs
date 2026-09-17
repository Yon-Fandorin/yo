use jiff::Timestamp;
use serde_json::{Map, Value};
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
};

use super::{
    error::{
        KimiAccountCapacityError, KimiAccountCapacityFailureKind, failure, limit_failure,
        protocol_failure,
    },
    model::{
        DecodedUsageRow, FIXED_POINT_PER_CENT, MAX_LIMIT_ROWS, MAX_NAME_BYTES, MAX_RESPONSE_BYTES,
        MINUTES_PER_WEEK,
    },
};
use crate::catalog::KimiCatalogSeed;

pub fn parse_kimi_account_capacity_snapshot(
    seed: &KimiCatalogSeed,
    bytes: &[u8],
) -> Result<AccountCapacitySnapshot, KimiAccountCapacityError> {
    parse_kimi_account_capacity_snapshot_with_plan(seed, bytes, None)
}

pub(super) fn parse_kimi_account_capacity_snapshot_with_plan(
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
        primary.map(|row| row.window()),
        secondary.map(|row| row.window()),
        credits,
        main_reason,
    ));
    for (index, row) in limits.into_iter().enumerate() {
        let (name, window, exhausted) = row.into_parts();
        let reason = exhausted.then(|| "usage_limit_reached".to_owned());
        buckets.push(AccountCapacityBucket::new(
            Some(format!("kimi-limit-{}", index + 2)),
            name,
            plan.clone(),
            Some(window),
            None,
            None,
            reason,
        ));
    }

    Ok(AccountCapacitySnapshot::new(
        seed.provider().clone(),
        seed.account().clone(),
        buckets,
    ))
}

pub(super) fn parse_kimi_account_plan(bytes: &[u8]) -> Result<String, KimiAccountCapacityError> {
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

pub(super) fn require_code_membership(
    seed: &KimiCatalogSeed,
) -> Result<(), KimiAccountCapacityError> {
    if seed.is_code_membership() {
        Ok(())
    } else {
        Err(failure(
            KimiAccountCapacityFailureKind::Configuration,
            "account capacity is supported only for a stored Kimi Code Membership account",
        ))
    }
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
    Ok(DecodedUsageRow::new(
        optional_name(object.get("name"))?.or(fallback_name),
        window,
        used >= limit,
    ))
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
        .any(DecodedUsageRow::exhausted)
        .then(|| "usage_limit_reached".to_owned())
}
