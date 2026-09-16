use serde_json::json;
use yo_core::{
    BackendFailure, BackendFailureKind, CacheReadInputTokens, ImageSummarySource,
    ModelConnectorInputItem, ModelConnectorInputRole, ModelReplayItem, ModelReplayRole,
};

use super::super::{NativeModelBackend, failure};

// 추론 항목이 의미 있는 단일 요약 메시지보다 앞에 올 수 있으므로, 해당 메시지를
// output slot 0으로 가정하지 않고 provider 출력 슬롯과 항목 identity로 연결합니다.
pub(super) fn bind_summary_message(
    identity: &mut Option<(usize, String)>,
    output_index: usize,
    item_id: &str,
) -> bool {
    match identity {
        Some((index, id)) => *index == output_index && id == item_id,
        None => {
            *identity = Some((output_index, item_id.to_owned()));
            true
        },
    }
}

impl NativeModelBackend {
    pub(super) fn context_summary_usage(
        &self,
        response_id: &str,
        round: usize,
        usage: &yo_core::ModelConnectorUsage,
    ) -> Result<serde_json::Value, BackendFailure> {
        let (Some(input_tokens), Some(output_tokens), Some(total_tokens)) =
            (usage.input_tokens, usage.output_tokens, usage.total_tokens)
        else {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: context summary usage is incomplete",
            ));
        };
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
        Ok(json!({
            "schema": "yo.model-usage-receipt/v1",
            "response_id": response_id,
            "round": u64::try_from(round).unwrap_or(u64::MAX),
            "provider": self.binding.provider_id().as_str(),
            "account": self.binding.account_id().as_str(),
            "model": self.binding.model_id().as_str(),
            "connector": self.binding.connector_id().as_str(),
            "api_dialect": self.binding.api_dialect().as_str(),
            "base_url": self.binding.endpoint().as_str(),
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "total_tokens": total_tokens,
                "reasoning_tokens": usage.reasoning_tokens,
            },
            "cache_read_input_tokens": cache_read_input_tokens,
        }))
    }
}

// 요약 원본은 provider 전용 assistant/tool replay가 아닌 불변 데이터입니다. 특히
// 이 projection은 private assistant envelope을 요구하거나 전달해서는 안 됩니다.
pub(super) fn summary_source(
    groups: &[Vec<ModelReplayItem>],
) -> Result<Option<ModelConnectorInputItem>, BackendFailure> {
    if groups
        .iter()
        .flatten()
        .any(|item| matches!(item, ModelReplayItem::MultimodalUser { .. }))
    {
        return ImageSummarySource::from_replay_groups(groups)
            .map(|source| Some(ModelConnectorInputItem::ImageSummarySource { source }))
            .map_err(|detail| failure(BackendFailureKind::ContextExhausted, detail));
    }
    const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
    let mut content = String::from("{\"history\":[");
    let mut count = 0;
    for item in groups.iter().flatten() {
        let record = match item {
            ModelReplayItem::Message {
                role,
                content,
                refusal,
            } => {
                let role = match role {
                    ModelReplayRole::System => "system",
                    ModelReplayRole::Developer => "developer",
                    ModelReplayRole::User => "user",
                    ModelReplayRole::Assistant => "assistant",
                };
                json!({"type":"message", "role":role, "content":content, "refusal":refusal})
            },
            ModelReplayItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                json!({"type":"function_call", "call_id":call_id, "name":name, "arguments":arguments})
            },
            ModelReplayItem::FunctionCallOutput { call_id, output } => {
                json!({"type":"function_call_output", "call_id":call_id, "output":output})
            },
            ModelReplayItem::ProviderPrivateAssistant { .. } => continue,
            ModelReplayItem::MultimodalUser { .. } => {
                unreachable!("image summaries use the typed source")
            },
        };
        let encoded = record.to_string();
        if content
            .len()
            .saturating_add(encoded.len())
            .saturating_add(2 + usize::from(count > 0))
            > MAX_SOURCE_BYTES
        {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: encoded visible summary source exceeds the 16-MiB message limit",
            ));
        }
        if count > 0 {
            content.push(',');
        }
        content.push_str(&encoded);
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }
    content.push_str("]}");
    Ok(Some(ModelConnectorInputItem::Message {
        role: ModelConnectorInputRole::User,
        content,
        refusal: None,
    }))
}
