use serde_json::{Value, json};
use yo_core::BackendFailure;

use crate::protocol;

pub(super) fn prompt_usage_receipt(
    result: &Value,
    response_id: u64,
) -> Result<Option<Value>, BackendFailure> {
    if let Some(usage) = result.get("usage") {
        return standard_prompt_usage_receipt(usage, response_id).map(Some);
    }

    let Some(usage) = result.get("_meta").and_then(|meta| meta.get("usage")) else {
        return Ok(None);
    };
    grok_meta_prompt_usage_receipt(usage, response_id)
}

fn standard_prompt_usage_receipt(usage: &Value, response_id: u64) -> Result<Value, BackendFailure> {
    require_usage_object(usage, "usage")?;
    Ok(json!({
        "schema": "grok.acp-prompt-usage-receipt/v1",
        "source_profile": "grok.acp.prompt-response.usage/v1",
        "prompt_request_id": response_id,
        "usage": {
            "input_tokens": non_negative_usage_at(usage, "inputTokens")?,
            "output_tokens": non_negative_usage_at(usage, "outputTokens")?,
            "total_tokens": non_negative_usage_at(usage, "totalTokens")?,
            "reasoning_tokens": non_negative_usage_at(usage, "thoughtTokens")?,
            "cache_read_input_tokens": non_negative_usage_at(usage, "cachedReadTokens")?,
            "cache_write_input_tokens": non_negative_usage_at(usage, "cachedWriteTokens")?,
        }
    }))
}

fn grok_meta_prompt_usage_receipt(
    usage: &Value,
    response_id: u64,
) -> Result<Option<Value>, BackendFailure> {
    require_usage_object(usage, "_meta.usage")?;
    let incomplete = match usage.get("usageIsIncomplete") {
        None => false,
        Some(Value::Bool(incomplete)) => *incomplete,
        Some(_) => {
            return Err(protocol::protocol_failure(
                "Grok ACP prompt _meta.usage `usageIsIncomplete` is not a boolean",
            ));
        },
    };
    if incomplete {
        return Ok(None);
    }
    let input_tokens = non_negative_usage_at(usage, "inputTokens")?;
    let output_tokens = non_negative_usage_at(usage, "outputTokens")?;
    let total_tokens = non_negative_usage_at(usage, "totalTokens")?;
    let reasoning_tokens = non_negative_usage_at(usage, "reasoningTokens")?;
    let cache_read_input_tokens = non_negative_usage_at(usage, "cachedReadTokens")?;
    let cache_write_input_tokens = non_negative_usage_at(usage, "cacheCreationTokens")?;
    let model_calls = non_negative_usage_at(usage, "modelCalls")?;
    let num_turns = non_negative_usage_at(usage, "numTurns")?;
    Ok(Some(json!({
        "schema": "grok.acp-prompt-usage-receipt/v1alpha1",
        "source_profile": "grok.acp.prompt-response.meta-usage/v1",
        "prompt_request_id": response_id,
        "model_calls": model_calls,
        "num_turns": num_turns,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "total_tokens": total_tokens,
            "reasoning_tokens": reasoning_tokens,
            "cache_read_input_tokens": cache_read_input_tokens,
            "cache_write_input_tokens": cache_write_input_tokens,
        }
    })))
}

fn require_usage_object(usage: &Value, field: &'static str) -> Result<(), BackendFailure> {
    if usage.is_object() {
        Ok(())
    } else {
        Err(protocol::protocol_failure(format!(
            "Grok ACP prompt {field} is not an object"
        )))
    }
}

fn non_negative_usage_at(usage: &Value, field: &'static str) -> Result<u64, BackendFailure> {
    usage.get(field).and_then(Value::as_u64).ok_or_else(|| {
        protocol::protocol_failure(format!(
            "Grok ACP prompt usage `{field}` is not non-negative"
        ))
    })
}
