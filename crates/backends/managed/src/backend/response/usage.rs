//! 사용량 귀속과 캐시 읽기 증거.

use serde_json::json;
use yo_core::{ActivityKind, ActivityOutcome, CacheReadInputTokens, ModelConnectorUsage};

use super::super::{NativeModelBackend, TurnState};

pub(super) fn record(
    backend: &mut NativeModelBackend,
    state: &mut TurnState,
    response_id: &str,
    usage: &ModelConnectorUsage,
) -> Result<(), yo_core::BackendFailure> {
    let attribution = backend.next_activity(state.turn)?;
    let cache_read_input_tokens = match &usage.cache_read_input_tokens {
        CacheReadInputTokens::Reported {
            tokens,
            source_profile,
        } => json!({
            "availability": "reported",
            "tokens": tokens,
            "source_profile": source_profile.as_str(),
        }),
        CacheReadInputTokens::Absent { source_profile } => json!({
            "availability": "absent",
            "source_profile": source_profile.as_str(),
        }),
        CacheReadInputTokens::Unsupported => json!({
            "availability": "unsupported",
        }),
    };
    backend.queue_activity_text(
        attribution,
        ActivityKind::ModelWork,
        json!({
            "schema": "yo.model-usage-receipt/v1",
            "response_id": response_id,
            "round": state.round,
            "provider": backend.binding.provider_id().as_str(),
            "account": backend.binding.account_id().as_str(),
            "model": backend.binding.model_id().as_str(),
            "connector": backend.binding.connector_id().as_str(),
            "api_dialect": backend.binding.api_dialect().as_str(),
            "base_url": backend.binding.endpoint().as_str(),
            "usage": {
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "total_tokens": usage.total_tokens,
                "reasoning_tokens": usage.reasoning_tokens,
            },
            "cache_read_input_tokens": cache_read_input_tokens,
        })
        .to_string(),
        Some(ActivityOutcome::Completed),
    );
    Ok(())
}
