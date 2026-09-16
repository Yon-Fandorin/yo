use serde_json::Value;

use super::{
    CODEX_USAGE_SCHEMA, GROK_USAGE_DIAGNOSTIC_SCHEMA, GROK_USAGE_SCHEMA, MANAGED_USAGE_SCHEMA,
    SessionUsage, SessionUsageError, SessionUsageReceipt, SessionUsageSource, UsageValue,
};
use crate::ActivityRef;

pub(super) fn parse_receipt(
    text: &str,
    activity: ActivityRef,
) -> Result<Option<SessionUsageReceipt>, SessionUsageError> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Ok(None);
    };
    let Some(schema) = value.get("schema").and_then(Value::as_str) else {
        return Ok(None);
    };
    let result = match schema {
        MANAGED_USAGE_SCHEMA => parse_managed(&value),
        GROK_USAGE_SCHEMA => parse_grok(&value),
        GROK_USAGE_DIAGNOSTIC_SCHEMA => parse_grok_diagnostic(&value),
        CODEX_USAGE_SCHEMA => parse_codex(&value),
        _ => return Ok(None),
    };
    result
        .map(|(source, usage)| {
            Some(SessionUsageReceipt {
                activity,
                source,
                usage,
            })
        })
        .map_err(|detail| SessionUsageError {
            activity,
            schema: schema.to_owned(),
            detail,
        })
}

pub(super) fn parse_managed(value: &Value) -> Result<(SessionUsageSource, SessionUsage), String> {
    let root = closed_object(
        value,
        "managed receipt",
        &[
            "schema",
            "response_id",
            "round",
            "provider",
            "account",
            "model",
            "connector",
            "api_dialect",
            "base_url",
            "usage",
            "cache_read_input_tokens",
        ],
    )?;
    let usage = closed_object_at(
        root,
        "usage",
        &[
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "reasoning_tokens",
        ],
    )?;
    Ok((
        SessionUsageSource::Managed {
            response_id: required_string(value, "response_id")?,
            round: required_root_u64(value, "round")?,
            provider: required_string(value, "provider")?,
            account: required_string(value, "account")?,
            model: required_string(value, "model")?,
            connector: required_string(value, "connector")?,
            api_dialect: required_string(value, "api_dialect")?,
            base_url: required_string(value, "base_url")?,
        },
        SessionUsage {
            input_tokens: optional_usage(usage, "input_tokens")?,
            output_tokens: optional_usage(usage, "output_tokens")?,
            total_tokens: optional_usage(usage, "total_tokens")?,
            reasoning_tokens: optional_usage(usage, "reasoning_tokens")?,
            cache_read_input_tokens: parse_managed_cache(value.get("cache_read_input_tokens"))?,
            cache_write_input_tokens: UsageValue::Unsupported,
        },
    ))
}

pub(super) fn parse_managed_cache(value: Option<&Value>) -> Result<UsageValue, String> {
    let Some(value) = value else {
        return Err("cache_read_input_tokens must be present".to_owned());
    };
    let object = value
        .as_object()
        .ok_or_else(|| "cache_read_input_tokens must be an object".to_owned())?;
    let availability = object
        .get("availability")
        .and_then(Value::as_str)
        .ok_or_else(|| "cache_read_input_tokens.availability must be a string".to_owned())?;
    match availability {
        "reported" => {
            validate_closed_fields(
                object,
                "cache_read_input_tokens",
                &["availability", "tokens", "source_profile"],
            )?;
            required_profile_id(object, "source_profile")?;
            Ok(UsageValue::Reported(
                object
                    .get("tokens")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        "cache_read_input_tokens.tokens must be a non-negative integer".to_owned()
                    })?,
            ))
        },
        "absent" => {
            validate_closed_fields(
                object,
                "cache_read_input_tokens",
                &["availability", "source_profile"],
            )?;
            required_profile_id(object, "source_profile")?;
            Ok(UsageValue::Absent)
        },
        "unsupported" => {
            validate_closed_fields(object, "cache_read_input_tokens", &["availability"])?;
            Ok(UsageValue::Unsupported)
        },
        availability => Err(format!(
            "unsupported cache_read_input_tokens availability {availability:?}"
        )),
    }
}

pub(super) fn parse_grok(value: &Value) -> Result<(SessionUsageSource, SessionUsage), String> {
    let usage = object_at(value, "usage")?;
    parse_grok_fields(value, usage, None, None)
}

pub(super) fn parse_grok_diagnostic(
    value: &Value,
) -> Result<(SessionUsageSource, SessionUsage), String> {
    let root = closed_object(
        value,
        "Grok diagnostic receipt",
        &[
            "schema",
            "source_profile",
            "prompt_request_id",
            "model_calls",
            "num_turns",
            "usage",
        ],
    )?;
    let usage = closed_object_at(
        root,
        "usage",
        &[
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "reasoning_tokens",
            "cache_read_input_tokens",
            "cache_write_input_tokens",
        ],
    )?;
    parse_grok_fields(
        value,
        usage,
        Some(required_root_u64(value, "model_calls")?),
        Some(required_root_u64(value, "num_turns")?),
    )
}

pub(super) fn parse_grok_fields(
    value: &Value,
    usage: &serde_json::Map<String, Value>,
    model_calls: Option<u64>,
    num_turns: Option<u64>,
) -> Result<(SessionUsageSource, SessionUsage), String> {
    let source_profile = required_string(value, "source_profile")?;
    let prompt_request_id = required_root_u64(value, "prompt_request_id")?;
    let source = match (model_calls, num_turns) {
        (None, None) => SessionUsageSource::Grok {
            source_profile,
            prompt_request_id,
        },
        (Some(model_calls), Some(num_turns)) => SessionUsageSource::GrokDiagnostic {
            source_profile,
            prompt_request_id,
            model_calls,
            num_turns,
        },
        _ => {
            return Err(
                "Grok diagnostic work counts must be both present or both absent".to_owned(),
            );
        },
    };
    Ok((
        source,
        SessionUsage {
            input_tokens: required_usage(usage, "input_tokens")?,
            output_tokens: required_usage(usage, "output_tokens")?,
            total_tokens: required_usage(usage, "total_tokens")?,
            reasoning_tokens: required_usage(usage, "reasoning_tokens")?,
            cache_read_input_tokens: required_usage(usage, "cache_read_input_tokens")?,
            cache_write_input_tokens: required_usage(usage, "cache_write_input_tokens")?,
        },
    ))
}

pub(super) fn parse_codex(value: &Value) -> Result<(SessionUsageSource, SessionUsage), String> {
    let usage = object_at(value, "usage")?;
    Ok((
        SessionUsageSource::Codex {
            source_profile: required_string(value, "source_profile")?,
            turn_id: required_string(value, "turn_id")?,
            model_context_window: optional_root_u64(value, "model_context_window")?,
        },
        SessionUsage {
            input_tokens: required_usage(usage, "input_tokens")?,
            output_tokens: required_usage(usage, "output_tokens")?,
            total_tokens: required_usage(usage, "total_tokens")?,
            reasoning_tokens: required_usage(usage, "reasoning_tokens")?,
            cache_read_input_tokens: required_usage(usage, "cache_read_input_tokens")?,
            cache_write_input_tokens: required_usage(usage, "cache_write_input_tokens")?,
        },
    ))
}

fn object_at<'a>(
    value: &'a Value,
    field: &str,
) -> Result<&'a serde_json::Map<String, Value>, String> {
    value
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{field} must be an object"))
}

fn closed_object<'a>(
    value: &'a Value,
    name: &str,
    allowed: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{name} must be an object"))?;
    validate_closed_fields(object, name, allowed)?;
    Ok(object)
}

fn closed_object_at<'a>(
    value: &'a serde_json::Map<String, Value>,
    field: &str,
    allowed: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, String> {
    let object = value
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{field} must be an object"))?;
    validate_closed_fields(object, field, allowed)?;
    Ok(object)
}

pub(super) fn validate_closed_fields(
    value: &serde_json::Map<String, Value>,
    name: &str,
    allowed: &[&str],
) -> Result<(), String> {
    if let Some(field) = value
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(format!("{name} has unsupported field {field:?}"));
    }
    Ok(())
}

pub(super) fn required_profile_id(
    value: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<(), String> {
    let profile = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cache_read_input_tokens.{field} must be a string"))?;
    crate::VersionedProfileId::new(profile)
        .map(|_| ())
        .map_err(|_| format!("cache_read_input_tokens.{field} must be a versioned profile ID"))
}

pub(super) fn required_usage(
    value: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<UsageValue, String> {
    Ok(UsageValue::Reported(required_u64(value, field)?))
}

pub(super) fn optional_usage(
    value: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<UsageValue, String> {
    let Some(raw) = value.get(field) else {
        return Ok(UsageValue::Absent);
    };
    if raw.is_null() {
        return Ok(UsageValue::Absent);
    }
    required_usage(value, field)
}

pub(super) fn required_u64(
    value: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{field} must be a non-negative integer"))
}

pub(super) fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{field} must be a string"))
}

pub(super) fn required_root_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{field} must be a non-negative integer"))
}

pub(super) fn optional_root_u64(value: &Value, field: &str) -> Result<Option<u64>, String> {
    let Some(raw) = value.get(field) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    raw.as_u64()
        .map(Some)
        .ok_or_else(|| format!("{field} must be a non-negative integer"))
}
