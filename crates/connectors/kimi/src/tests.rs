use serde_json::json;
use yo_core::{
    CompleteModelBinding, FunctionTool, ModelCacheAffinityHint, ModelConnectorEvent,
    ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorLimits, ModelConnectorRequest,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ReasoningEffort,
    RequestToolExposure,
};

use crate::{
    private_replay::{
        KimiAssistantMessage, KimiAssistantToolCall, decode_envelope, encode_envelope,
        kimi_replay_round_item_lengths,
    },
    request::{KimiWireKind, admit_binding, wire_body},
    sse::ChatCompletionsSseDecoder,
};

fn complete(
    model: &str,
    input: u64,
    output: u64,
    reasoning: &str,
    optional: &str,
    replay: &str,
) -> CompleteModelBinding {
    complete_at(
        "https://api.moonshot.ai/v1",
        model,
        input,
        output,
        reasoning,
        optional,
        replay,
    )
}

fn complete_at(
    endpoint: &str,
    model: &str,
    input: u64,
    output: u64,
    reasoning: &str,
    optional: &str,
    replay: &str,
) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"kimi","account":"default","model":"{model}","connector":"kimi-chat-completions","base_url":"{endpoint}","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":{input},"max_output_tokens":{output},"reasoning_parameters":{reasoning},"optional_request_parameters":{optional},"tool_capability_policy":"local-tools/v1","replay_profile":"{replay}"}}"#
    ))
    .unwrap()
}

fn k3() -> CompleteModelBinding {
    complete(
        "kimi-k3",
        1_048_576,
        131_072,
        r#"{"effort":"max"}"#,
        "{}",
        "kimi-private-local-plaintext/v1",
    )
}

fn event(value: serde_json::Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(&value).unwrap())
}

fn fixture_session(value: u64) -> yo_core::SessionId {
    format!("01890f00-0000-7000-8000-{value:012x}")
        .parse()
        .unwrap()
}

fn body_for_complete(
    complete: &CompleteModelBinding,
    effort: Option<ReasoningEffort>,
) -> serde_json::Value {
    let mut request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        complete.profile().context().max_output_tokens().unwrap(),
        effort,
    )
    .unwrap();
    if complete.binding().endpoint().as_str() == "https://api.kimi.com/coding/v1" {
        request = request
            .with_cache_affinity_hint(ModelCacheAffinityHint::for_session(fixture_session(41)));
    }
    wire_body(
        &request,
        complete.binding().model_id().as_str(),
        admit_binding(complete).unwrap(),
    )
    .unwrap()
}

mod replay;
mod request;
mod stream;
