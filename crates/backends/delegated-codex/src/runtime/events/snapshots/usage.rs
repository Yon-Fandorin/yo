use serde_json::{Value, json};
use yo_core::BackendFailure;

use crate::protocol;

#[derive(Clone, Copy)]
pub(in crate::runtime::events) struct TokenUsageBreakdown {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    reasoning_tokens: u64,
    cache_read_input_tokens: u64,
    cache_write_input_tokens: u64,
}

impl TokenUsageBreakdown {
    pub(in crate::runtime::events) fn to_json(self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
            "reasoning_tokens": self.reasoning_tokens,
            "cache_read_input_tokens": self.cache_read_input_tokens,
            "cache_write_input_tokens": self.cache_write_input_tokens,
        })
    }
}

pub(in crate::runtime::events) fn token_usage_breakdown_at(
    value: &Value,
    field: &'static str,
) -> Result<TokenUsageBreakdown, BackendFailure> {
    let value = value_at(value, &[field], "token usage breakdown")?;
    Ok(TokenUsageBreakdown {
        input_tokens: non_negative_at(value, "inputTokens", "input tokens")?,
        output_tokens: non_negative_at(value, "outputTokens", "output tokens")?,
        total_tokens: non_negative_at(value, "totalTokens", "total tokens")?,
        reasoning_tokens: non_negative_at(
            value,
            "reasoningOutputTokens",
            "reasoning output tokens",
        )?,
        cache_read_input_tokens: non_negative_at(
            value,
            "cachedInputTokens",
            "cached input tokens",
        )?,
        cache_write_input_tokens: optional_non_negative_at(
            value,
            "cacheWriteInputTokens",
            "cache write input tokens",
        )?
        .unwrap_or(0),
    })
}

pub(in crate::runtime::events) fn value_at<'a>(
    value: &'a Value,
    path: &[&str],
    label: &'static str,
) -> Result<&'a Value, BackendFailure> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or_else(|| {
            protocol::protocol_failure(format!("Codex message is missing {label}"))
        })?;
    }
    if !current.is_object() {
        return Err(protocol::protocol_failure(format!(
            "Codex {label} is not an object"
        )));
    }
    Ok(current)
}

fn non_negative_at(
    value: &Value,
    field: &'static str,
    label: &'static str,
) -> Result<u64, BackendFailure> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| protocol::protocol_failure(format!("Codex {label} is not non-negative")))
}

pub(in crate::runtime::events) fn optional_non_negative_at(
    value: &Value,
    field: &'static str,
    label: &'static str,
) -> Result<Option<u64>, BackendFailure> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            protocol::protocol_failure(format!("Codex {label} is not non-negative"))
        }),
    }
}
