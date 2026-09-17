use serde_json::Value;
use yo_core::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountId,
    ModelServiceError, ProviderId,
};

use super::{
    error::{QwenCloudCapacityFailureKind, QwenCloudResult, expired_session_error, failure},
    model::{ExpectedMedia, QwenCloudProviderData, QwenCloudQuotaData, QwenCloudUsageData},
};

const MAX_PLAN_BYTES: usize = 128;
const FIVE_HOURS_MINUTES: u64 = 5 * 60;
const ONE_WEEK_MINUTES: u64 = 7 * 24 * 60;

pub(super) fn decode_gateway_envelope(bytes: &[u8]) -> QwenCloudResult<Value> {
    let root: Value = serde_json::from_slice(bytes).map_err(|_| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            "QwenCloud console response is not valid JSON",
        )
    })?;
    let root_code = root.get("code");
    if root_code.and_then(Value::as_str) == Some("ConsoleNeedLogin") {
        return Err(expired_session_error());
    }
    let root_success = root_code.and_then(Value::as_str) == Some("200")
        || root_code.and_then(Value::as_u64) == Some(200);
    let inner = root.pointer("/data/DataV2/data").ok_or_else(|| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            "QwenCloud console response has no gateway result",
        )
    })?;
    if !root_success
        || inner.get("code").and_then(Value::as_str) != Some("SUCCESS")
        || inner.get("success").and_then(Value::as_bool) != Some(true)
    {
        return Err(failure(
            QwenCloudCapacityFailureKind::Protocol,
            "QwenCloud console rejected the capacity request; the browser session may have expired",
        ));
    }
    inner.get("data").cloned().ok_or_else(|| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            "QwenCloud console response has no capacity payload",
        )
    })
}

pub(super) fn decode_snapshot(
    usage: &Value,
    subscription: &Value,
    quota_config: &Value,
    provider: &ProviderId,
    account: &AccountId,
) -> QwenCloudResult<(AccountCapacitySnapshot, QwenCloudProviderData)> {
    let spec_code = bounded_text(subscription.get("specCode"), "subscription specCode")?;
    let tier = quota_config
        .get(spec_code)
        .and_then(Value::as_object)
        .ok_or_else(|| {
            failure(
                QwenCloudCapacityFailureKind::Protocol,
                "QwenCloud quota configuration has no active subscription tier",
            )
        })?;
    let weekly_quota = validate_positive_number(tier.get("weekly"), "weekly quota")?;
    if tier.contains_key("five_hour") {
        validate_positive_number(tier.get("five_hour"), "five-hour quota")?;
    }
    let five_hour = decode_window(usage, "5Hour", FIVE_HOURS_MINUTES)?;
    let weekly = decode_window(usage, "1Week", ONE_WEEK_MINUTES)?;
    let (primary, secondary) = match (five_hour, weekly) {
        (Some(five_hour), Some(weekly)) => (Some(five_hour), Some(weekly)),
        (Some(five_hour), None) => (Some(five_hour), None),
        (None, Some(weekly)) => (Some(weekly), None),
        (None, None) => {
            return Err(failure(
                QwenCloudCapacityFailureKind::Protocol,
                "QwenCloud usage response contains no usable quota window",
            ));
        },
    };
    let limited = primary
        .iter()
        .chain(secondary.iter())
        .any(|window| window.used_percent_basis_points() == 10_000);
    let bucket = AccountCapacityBucket::new(
        Some("qwencloud".to_owned()),
        Some("QwenCloud Token Plan".to_owned()),
        Some(spec_code.to_owned()),
        primary,
        secondary,
        None,
        limited.then(|| "usage_limit_reached".to_owned()),
    );
    let provider_data = QwenCloudProviderData {
        spec_code: spec_code.to_owned(),
        usage: QwenCloudUsageData {
            per5_hour_percentage: five_hour
                .as_ref()
                .and_then(|_| usage.get("per5HourPercentage").cloned()),
            per5_hour_reset_time: five_hour
                .as_ref()
                .and_then(|_| usage.get("per5HourResetTime").cloned()),
            per1_week_percentage: weekly
                .as_ref()
                .and_then(|_| usage.get("per1WeekPercentage").cloned()),
            per1_week_reset_time: weekly
                .as_ref()
                .and_then(|_| usage.get("per1WeekResetTime").cloned()),
        },
        quota: QwenCloudQuotaData {
            five_hour: tier.get("five_hour").cloned(),
            weekly: weekly_quota.clone(),
        },
    };
    Ok((
        AccountCapacitySnapshot::new(provider.clone(), account.clone(), vec![bucket]),
        provider_data,
    ))
}

fn decode_window(
    usage: &Value,
    field_prefix: &str,
    duration_minutes: u64,
) -> QwenCloudResult<Option<AccountCapacityWindow>> {
    let percentage_field = format!("per{field_prefix}Percentage");
    let reset_field = format!("per{field_prefix}ResetTime");
    let Some(raw_percentage) = usage.get(&percentage_field) else {
        return Ok(None);
    };
    let percentage = number(raw_percentage).ok_or_else(|| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            format!("QwenCloud usage {percentage_field} is not numeric"),
        )
    })?;
    if !percentage.is_finite() || !(0.0..=1.0).contains(&percentage) {
        return Err(failure(
            QwenCloudCapacityFailureKind::Protocol,
            format!("QwenCloud usage {percentage_field} is outside 0..1"),
        ));
    }
    let used_percent_basis_points = (percentage * 10_000.0).ceil().min(10_000.0) as u16;
    let reset = usage
        .get(&reset_field)
        .map(|value| normalize_reset(value, &reset_field))
        .transpose()?;
    AccountCapacityWindow::from_used_percent_basis_points(
        used_percent_basis_points,
        Some(duration_minutes),
        reset,
    )
    .map(Some)
    .map_err(model_service_error)
}

fn normalize_reset(value: &Value, field: &str) -> QwenCloudResult<i64> {
    let value = value.as_i64().ok_or_else(|| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            format!("QwenCloud usage {field} is not an integer"),
        )
    })?;
    if value <= 0 {
        return Err(failure(
            QwenCloudCapacityFailureKind::Protocol,
            format!("QwenCloud usage {field} must be positive"),
        ));
    }
    Ok(if value >= 100_000_000_000 {
        value / 1_000
    } else {
        value
    })
}

fn bounded_text<'a>(value: Option<&'a Value>, field: &str) -> QwenCloudResult<&'a str> {
    let value = value.and_then(Value::as_str).ok_or_else(|| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            format!("QwenCloud {field} is not a string"),
        )
    })?;
    if value.is_empty() || value.len() > MAX_PLAN_BYTES || value.chars().any(char::is_control) {
        return Err(failure(
            QwenCloudCapacityFailureKind::Limit,
            format!("QwenCloud {field} is outside the bounded text profile"),
        ));
    }
    Ok(value)
}

fn validate_positive_number<'a>(
    value: Option<&'a Value>,
    field: &str,
) -> QwenCloudResult<&'a Value> {
    let source = value.ok_or_else(|| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            format!("QwenCloud {field} is not numeric"),
        )
    })?;
    let value = number(source).ok_or_else(|| {
        failure(
            QwenCloudCapacityFailureKind::Protocol,
            format!("QwenCloud {field} is not numeric"),
        )
    })?;
    if !value.is_finite() || value <= 0.0 {
        return Err(failure(
            QwenCloudCapacityFailureKind::Protocol,
            format!("QwenCloud {field} must be finite and positive"),
        ));
    }
    Ok(source)
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

pub(super) fn extract_sec_token(html: &str) -> Option<&str> {
    for marker in ["SEC_TOKEN", "SEC-TOKEN"] {
        let mut rest = html;
        while let Some(index) = rest.find(marker) {
            rest = &rest[index + marker.len()..];
            let probe = &rest[..rest.len().min(512)];
            let Some(separator) = probe.find([':', '=']) else {
                continue;
            };
            let candidate = rest[separator + 1..].trim_start();
            let Some(quote) = candidate.chars().next() else {
                continue;
            };
            if quote != '\'' && quote != '"' {
                continue;
            }
            let quoted = &candidate[quote.len_utf8()..];
            let Some(end) = quoted.find(quote) else {
                continue;
            };
            let token = &quoted[..end];
            if !token.is_empty() && !token.chars().any(char::is_control) {
                return Some(token);
            }
        }
    }
    None
}

pub(super) fn matches_media_type(content_type: &str, expected: ExpectedMedia) -> bool {
    let media_type = content_type.split(';').next().unwrap_or("").trim();
    match expected {
        ExpectedMedia::Html => media_type.eq_ignore_ascii_case("text/html"),
        ExpectedMedia::Json => {
            media_type.eq_ignore_ascii_case("application/json")
                || media_type
                    .to_ascii_lowercase()
                    .strip_prefix("application/")
                    .is_some_and(|subtype| subtype.ends_with("+json"))
        },
    }
}

fn model_service_error(error: ModelServiceError) -> super::error::QwenCloudCapacityError {
    failure(
        QwenCloudCapacityFailureKind::Protocol,
        format!("normalizing QwenCloud capacity: {error}"),
    )
}
