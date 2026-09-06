use serde_json::Value;
use yo_core::{ConnectorError, ModelConnectorUsage};

use super::protocol_failure;

#[cfg(test)]
mod tests;

pub(super) fn decode_usage(value: &Value) -> Result<ModelConnectorUsage, ConnectorError> {
    const CACHE_READ_SOURCE_PROFILE: &str = "kimi.usage.cached-tokens/v1";

    let prompt = non_negative_at(value, "prompt_tokens")?;
    let completion = non_negative_at(value, "completion_tokens")?;
    let total = non_negative_at(value, "total_tokens")?;
    if prompt.checked_add(completion) != Some(total) {
        return Err(protocol_failure(
            "Chat Completions usage total is inconsistent",
        ));
    }
    let reasoning = value
        .get("completion_tokens_details")
        .filter(|details| !details.is_null())
        .and_then(|details| details.get("reasoning_tokens"))
        .filter(|reasoning| !reasoning.is_null())
        .map(|reasoning| {
            reasoning.as_u64().ok_or_else(|| {
                protocol_failure("Chat Completions reasoning_tokens is not non-negative")
            })
        })
        .transpose()?;
    let cache_read_input_tokens = match value.get("cached_tokens") {
        Some(cached) => {
            let tokens = cached.as_u64().ok_or_else(|| {
                protocol_failure("Chat Completions cached_tokens is not non-negative")
            })?;
            if tokens > prompt {
                return Err(protocol_failure(
                    "Chat Completions cached_tokens exceeds prompt_tokens",
                ));
            }
            yo_core::CacheReadInputTokens::Reported {
                tokens,
                source_profile: yo_core::VersionedProfileId::new(CACHE_READ_SOURCE_PROFILE)
                    .expect("the closed Kimi usage source profile is valid"),
            }
        },
        None => yo_core::CacheReadInputTokens::Absent {
            source_profile: yo_core::VersionedProfileId::new(CACHE_READ_SOURCE_PROFILE)
                .expect("the closed Kimi usage source profile is valid"),
        },
    };
    Ok(ModelConnectorUsage {
        input_tokens: Some(prompt),
        output_tokens: Some(completion),
        total_tokens: Some(total),
        reasoning_tokens: reasoning,
        cache_read_input_tokens,
    })
}

fn non_negative_at(value: &Value, field: &'static str) -> Result<u64, ConnectorError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| protocol_failure(format!("Chat Completions {field} is not non-negative")))
}
