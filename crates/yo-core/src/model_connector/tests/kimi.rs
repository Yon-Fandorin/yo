use serde_json::json;

use super::super::{
    ResponsesConnectorLimits,
    chat_sse::ChatCompletionsSseDecoder,
    kimi_request::{admit_binding, wire_body},
    request::{
        FunctionTool, ModelCacheAffinityHint, ReasoningEffort, RequestToolExposure,
        ResponsesInputItem, ResponsesInputRole, ResponsesRequest,
    },
    types::ResponsesEvent,
};
use crate::{
    CompleteModelBinding, KimiAssistantMessage, KimiAssistantToolCall, ModelReplayDelta,
    ModelReplayItem, ModelReplayRole,
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

// K3 wire는 profile에서 확정한 output/reasoning만 보내고 deprecated sampling 및
// stream_options를 넣지 않아 다른 Chat Completions 의미를 우연히 상속하지 않습니다.
#[test]
fn k3_request_uses_only_the_closed_kimi_wire_fields() {
    let profile = admit_binding(&k3()).unwrap();
    let request = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        131_072,
        Some(ReasoningEffort::Max),
    )
    .unwrap();
    let body = wire_body(&request, "kimi-k3", profile).unwrap();
    assert_eq!(body["model"], "kimi-k3");
    assert_eq!(body["max_completion_tokens"], 131_072);
    assert_eq!(body["reasoning_effort"], "max");
    for omitted in [
        "stream_options",
        "thinking",
        "max_tokens",
        "temperature",
        "top_p",
    ] {
        assert!(body.get(omitted).is_none(), "{omitted}");
    }
}

// Kimi K3의 complete profile은 131072 hard max를 유지하지만 각 요청은 그 이하의
// 양수 4096을 보낼 수 있고, hard max 초과와 unknown은 transport 전에 거절됩니다.
#[test]
fn kimi_accepts_a_smaller_positive_cap_and_rejects_overflow_or_unknown() {
    let profile = admit_binding(&k3()).unwrap();
    let input = || {
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }]
    };
    let smaller = ResponsesRequest::new(
        input(),
        RequestToolExposure::disabled(),
        4_096,
        Some(ReasoningEffort::Max),
    )
    .unwrap();
    assert_eq!(
        wire_body(&smaller, "kimi-k3", profile).unwrap()["max_completion_tokens"],
        4_096
    );

    for cap in [Some(131_073), None] {
        let request = ResponsesRequest::new(
            input(),
            RequestToolExposure::disabled(),
            cap,
            Some(ReasoningEffort::Max),
        )
        .unwrap();
        assert!(wire_body(&request, "kimi-k3", profile).is_err());
    }
}

// 1M Code k3는 hard max 131072 이하의 request cap 4096, preserved-thinking, caller의 typed
// cache hint를 함께 보냅니다. hint가 없거나 endpoint/ModelId가 교차되면 transport 전에
// 실패합니다.
#[test]
fn code_k3_requires_preserved_thinking_and_typed_cache_affinity() {
    let complete = complete_at(
        "https://api.kimi.com/coding/v1",
        "k3",
        1_048_576,
        131_072,
        r#"{"effort":"high"}"#,
        r#"{"thinking":{"type":"enabled","keep":"all"}}"#,
        "kimi-private-local-plaintext/v1",
    );
    let profile = admit_binding(&complete).unwrap();
    let request = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        4_096,
        Some(ReasoningEffort::High),
    )
    .unwrap();
    assert!(wire_body(&request, "k3", profile).is_err());

    let session = crate::fixture_session(31);
    let request = request.with_cache_affinity_hint(ModelCacheAffinityHint::for_session(session));
    let body = wire_body(&request, "k3", profile).unwrap();
    assert_eq!(body["max_completion_tokens"], 4_096);
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["thinking"], json!({"type": "enabled", "keep": "all"}));
    assert_eq!(body["prompt_cache_key"], session.to_string());

    let crossed = complete_at(
        "https://api.kimi.com/coding/v1",
        "kimi-k3",
        1_048_576,
        131_072,
        r#"{"effort":"max"}"#,
        "{}",
        "kimi-private-local-plaintext/v1",
    );
    assert!(admit_binding(&crossed).is_err());
}

// Code K2.7도 같은 cache key와 keep-all을 사용하지만 reasoning_effort는 보내지 않으며,
// ordinary backend hint를 받은 Platform K3는 provider cache field를 직렬화하지 않습니다.
#[test]
fn cache_affinity_is_serialized_only_by_code_wire_variants() {
    let session = crate::fixture_session(32);
    let hint = ModelCacheAffinityHint::for_session(session);
    let code = complete_at(
        "https://api.kimi.com/coding/v1",
        "kimi-for-coding",
        262_144,
        32_768,
        "{}",
        r#"{"thinking":{"type":"enabled","keep":"all"}}"#,
        "kimi-private-local-plaintext/v1",
    );
    let code_request = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        32_768,
        None,
    )
    .unwrap()
    .with_cache_affinity_hint(hint.clone());
    let code_body = wire_body(
        &code_request,
        "kimi-for-coding",
        admit_binding(&code).unwrap(),
    )
    .unwrap();
    assert_eq!(code_body["prompt_cache_key"], session.to_string());
    assert_eq!(code_body["stream_options"]["include_usage"], true);
    assert!(code_body.get("reasoning_effort").is_none());

    let platform_request = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        131_072,
        Some(ReasoningEffort::Max),
    )
    .unwrap()
    .with_cache_affinity_hint(hint);
    let platform_body =
        wire_body(&platform_request, "kimi-k3", admit_binding(&k3()).unwrap()).unwrap();
    assert!(platform_body.get("prompt_cache_key").is_none());
}

// 저장된 generic assistant+function projection과 private Kimi item은 다음 요청에서 private
// assistant 한 개로 교체되어 reasoning을 보존하되 visible content/tool call을 중복 전송하지
// 않습니다.
#[test]
fn private_replay_replaces_its_visible_assistant_projection_once() {
    let profile = admit_binding(&k3()).unwrap();
    let private = KimiAssistantMessage::new(
        "hidden",
        Some("visible".to_owned()),
        vec![KimiAssistantToolCall::new(
            "call-1",
            "read_file",
            r#"{"path":"README.md"}"#,
        )],
    );
    let request = ResponsesRequest::new(
        vec![
            ResponsesInputItem::Message {
                role: ResponsesInputRole::Assistant,
                content: "visible".to_owned(),
                refusal: None,
            },
            ResponsesInputItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"README.md"}"#.to_owned(),
            },
            ResponsesInputItem::ProviderPrivateAssistant {
                schema: "kimi.assistant-message/v1alpha1".to_owned(),
                message: private,
            },
            ResponsesInputItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "contents".to_owned(),
            },
        ],
        RequestToolExposure::disabled(),
        131_072,
        Some(ReasoningEffort::Max),
    )
    .unwrap();
    let body = wire_body(&request, "kimi-k3", profile).unwrap();
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["reasoning_content"], "hidden");
    assert_eq!(messages[0]["content"], "visible");
    assert_eq!(messages[0]["tool_calls"].as_array().unwrap().len(), 1);
    assert_eq!(messages[1]["role"], "tool");
}

// private item이 visible projection과 호출 개수만 같고 ID·이름·인자 중 하나가 다르면
// 다음 Kimi 요청을 만들기 전에 거절해 다른 도구 실행 이력을 재전송하지 않습니다.
#[test]
fn private_replay_requires_every_projected_tool_call_field_to_match() {
    let profile = admit_binding(&k3()).unwrap();
    for private in [
        KimiAssistantToolCall::new("other-call", "read_file", r#"{"path":"README.md"}"#),
        KimiAssistantToolCall::new("call-1", "write_file", r#"{"path":"README.md"}"#),
        KimiAssistantToolCall::new("call-1", "read_file", r#"{"path":"other.md"}"#),
    ] {
        let request = ResponsesRequest::new(
            vec![
                ResponsesInputItem::Message {
                    role: ResponsesInputRole::Assistant,
                    content: String::new(),
                    refusal: None,
                },
                ResponsesInputItem::FunctionCall {
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"README.md"}"#.to_owned(),
                },
                ResponsesInputItem::ProviderPrivateAssistant {
                    schema: "kimi.assistant-message/v1alpha1".to_owned(),
                    message: KimiAssistantMessage::new("hidden", None, vec![private]),
                },
            ],
            RequestToolExposure::disabled(),
            131_072,
            Some(ReasoningEffort::Max),
        )
        .unwrap();
        let error = wire_body(&request, "kimi-k3", profile).unwrap_err();
        assert!(
            error
                .message()
                .contains("differs from its semantic projection"),
            "{error}"
        );
    }
}

// stop형 private replay는 tool call이 없을 때 content 문자열이 반드시 존재해야 하므로
// null을 빈 visible content와 같은 값으로 보내지 않습니다.
#[test]
fn private_replay_rejects_null_content_without_tool_calls() {
    let profile = admit_binding(&k3()).unwrap();
    let request = ResponsesRequest::new(
        vec![
            ResponsesInputItem::Message {
                role: ResponsesInputRole::Assistant,
                content: String::new(),
                refusal: None,
            },
            ResponsesInputItem::ProviderPrivateAssistant {
                schema: "kimi.assistant-message/v1alpha1".to_owned(),
                message: KimiAssistantMessage::new("", None, Vec::new()),
            },
        ],
        RequestToolExposure::disabled(),
        131_072,
        Some(ReasoningEffort::Max),
    )
    .unwrap();

    assert!(wire_body(&request, "kimi-k3", profile).is_err());
}

// Kimi strict tool은 JSON object 값이기만 한 schema가 아니라 canonical
// yo.tool-schema/v1의 object root와 Kimi MFJS subset을 모두 만족해야 합니다.
#[test]
fn strict_kimi_tools_require_the_canonical_object_schema_root() {
    let profile = admit_binding(&k3()).unwrap();
    for parameters in [
        json!({"type":"string"}),
        json!({"type":"object","additionalProperties":true}),
        json!({"type":"array","items":{"type":"string"}}),
    ] {
        let tool = FunctionTool::new("read_file", "read a file", parameters).unwrap();
        let request = ResponsesRequest::new(
            vec![ResponsesInputItem::Message {
                role: ResponsesInputRole::User,
                content: "hello".to_owned(),
                refusal: None,
            }],
            RequestToolExposure::enabled(vec![tool]),
            131_072,
            Some(ReasoningEffort::Max),
        )
        .unwrap();
        assert!(wire_body(&request, "kimi-k3", profile).is_err());
    }
}

// Kimi stream의 reasoning은 frontend event가 아니라 한 bounded private replay item으로만
// 나오고, visible content와 tool call을 같은 assistant message에 정확히 상관시킵니다.
#[test]
fn kimi_stream_keeps_reasoning_private_and_emits_exact_replay_item() {
    let stream = [
        event(json!({
            "id":"kimi-1","object":"chat.completion.chunk","model":"kimi-k3",
            "choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"hidden","content":"visible"},"finish_reason":null}]
        })),
        event(json!({
            "id":"kimi-1","object":"chat.completion.chunk","model":"kimi-k3",
            "choices":[{"index":0,"delta":{},"finish_reason":"stop","usage":{"prompt_tokens":4,"completion_tokens":3,"total_tokens":7}}]
        })),
        "data: [DONE]\n\n".to_owned(),
    ].concat();
    let mut decoder = ChatCompletionsSseDecoder::new_kimi(
        ResponsesConnectorLimits::default(),
        "kimi-k3".to_owned(),
        true,
    );
    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ResponsesEvent::ReasoningDelta { .. }))
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::ProviderPrivateAssistant { message, .. }
            if message.reasoning_content() == "hidden" && message.content() == Some("visible")
    )));
    assert_eq!(decoder.kimi_private_retained_lengths(), (0, 0, 0));
}

// K2.7 tool round는 분할된 reasoning/argument를 한 private assistant로 조립하고,
// finish choice·top-level·후속 empty-choice에 중복된 동일 usage를 한 의미로 받습니다.
#[test]
fn kimi_tool_round_accepts_every_equivalent_usage_placement_once() {
    let usage = json!({"prompt_tokens":4,"completion_tokens":3,"total_tokens":7});
    let stream = [
        event(json!({
            "id":"kimi-tool","object":"chat.completion.chunk","model":"kimi-k2.7-code",
            "choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"hid",
                "tool_calls":[{"index":0,"id":"call-1","type":"function",
                    "function":{"name":"read_file","arguments":"{\"path\":"}}]},"finish_reason":null}]
        })),
        event(json!({
            "id":"kimi-tool","object":"chat.completion.chunk","model":"kimi-k2.7-code",
            "usage":usage,
            "choices":[{"index":0,"delta":{"reasoning_content":"den",
                "tool_calls":[{"index":0,"function":{"arguments":"\"README.md\"}"}}]},
                "finish_reason":"tool_calls","usage":usage}]
        })),
        event(json!({
            "id":"kimi-tool","object":"chat.completion.chunk","model":"kimi-k2.7-code",
            "choices":[],"usage":usage
        })),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ChatCompletionsSseDecoder::new_kimi(
        ResponsesConnectorLimits::default(),
        "kimi-k2.7-code".to_owned(),
        true,
    );
    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());
    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::ProviderPrivateAssistant { message, .. }
            if message.reasoning_content() == "hidden"
                && message.content().is_none()
                && message.tool_calls().len() == 1
                && message.tool_calls()[0].arguments() == r#"{"path":"README.md"}"#
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ResponsesEvent::Terminal { .. }))
            .count(),
        1
    );
}

// stable model/first role/delta field/private-reasoning 규칙 중 하나라도 어기면 visible
// 일부가 있더라도 terminal이나 replay로 승격하지 않고 protocol failure로 닫습니다.
#[test]
fn kimi_stream_rejects_identity_role_and_private_shape_mismatches() {
    for chunk in [
        json!({"id":"x","object":"chat.completion.chunk","model":"other","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}),
        json!({"id":"x","object":"chat.completion.chunk","model":"kimi-k2.6","choices":[{"index":0,"delta":{"content":"missing role"},"finish_reason":null}]}),
        json!({"id":"x","object":"chat.completion.chunk","model":"kimi-k2.6","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"forbidden"},"finish_reason":null}]}),
        json!({"id":"x","object":"chat.completion.chunk","model":"kimi-k2.6","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":""},"finish_reason":null}]}),
    ] {
        let model = chunk["model"].as_str().unwrap().to_owned();
        let mut decoder = ChatCompletionsSseDecoder::new_kimi(
            ResponsesConnectorLimits::default(),
            if model == "other" {
                "kimi-k2.6".to_owned()
            } else {
                model
            },
            false,
        );
        assert!(decoder.push(event(chunk).as_bytes()).is_err());
    }
}

// Kimi private assistant의 canonical JSON 크기는 content와 reasoning을 따로 허용하지
// 않고 하나의 합산 상한으로, 초과 fragment를 문자열에 붙이기 전에 판정합니다.
#[test]
fn kimi_stream_enforces_one_exact_private_message_budget() {
    let first = json!({
        "id":"kimi-budget","object":"chat.completion.chunk","model":"kimi-k3",
        "choices":[{"index":0,"delta":{
            "role":"assistant","reasoning_content":"hidden","content":"visible"
        },"finish_reason":null}]
    });
    let exact = serde_json::to_vec(&json!({
        "role":"assistant","reasoning_content":"hidden","content":"visible"
    }))
    .unwrap()
    .len();
    let mut exact_decoder = ChatCompletionsSseDecoder::new_kimi(
        ResponsesConnectorLimits {
            max_provider_private_bytes: exact,
            ..ResponsesConnectorLimits::default()
        },
        "kimi-k3".to_owned(),
        true,
    );
    exact_decoder.push(event(first.clone()).as_bytes()).unwrap();

    let mut overflow = ChatCompletionsSseDecoder::new_kimi(
        ResponsesConnectorLimits {
            max_provider_private_bytes: exact - 1,
            ..ResponsesConnectorLimits::default()
        },
        "kimi-k3".to_owned(),
        true,
    );
    let error = overflow.push(event(first).as_bytes()).unwrap_err();
    assert_eq!(error.kind(), super::super::ConnectorFailureKind::Limit);
    let (content, reasoning, _) = overflow.kimi_private_retained_lengths();
    assert!(content < "visible".len() || reasoning < "hidden".len());
}

// backend가 round 시작 전에 넘긴 canonical 남은 replay budget은 Kimi의 visible projection과
// private 복제 비용을 함께 세어, 첫 초과 fragment를 decoder state와 event에서 모두 배제합니다.
#[test]
fn kimi_stream_rejects_the_first_complete_replay_overflow_fragment_before_retention() {
    let json_bytes = |value: &str| serde_json::to_string(value).unwrap().len() - 2;
    let exact_content = "visible";
    let exact_lengths =
        crate::kimi_replay_round_item_lengths(true, true, json_bytes(exact_content), 0, &[])
            .unwrap();
    let empty_prefix = ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: String::new(),
        refusal: None,
    };
    let empty_budget =
        ModelReplayDelta::replay_budget(None, std::iter::once(&empty_prefix)).unwrap();
    let fixed_bytes = empty_budget
        .encoded_len_with_item_lengths(&exact_lengths)
        .unwrap();
    let prefix = ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: "p".repeat(ModelReplayDelta::MAX_ENCODED_BYTES - fixed_bytes),
        refusal: None,
    };
    let replay_budget = ModelReplayDelta::replay_budget(None, std::iter::once(&prefix)).unwrap();
    assert!(replay_budget.accepts_item_lengths(&exact_lengths));
    let overflow_lengths = crate::kimi_replay_round_item_lengths(
        true,
        true,
        json_bytes(&format!("{exact_content}x")),
        0,
        &[],
    )
    .unwrap();
    assert!(!replay_budget.accepts_item_lengths(&overflow_lengths));

    let mut decoder = ChatCompletionsSseDecoder::new_kimi_with_replay_budget(
        ResponsesConnectorLimits::default(),
        "kimi-k3".to_owned(),
        true,
        replay_budget,
    );
    decoder
        .push(
            event(json!({
                "id":"combined-budget","object":"chat.completion.chunk","model":"kimi-k3",
                "choices":[{"index":0,"delta":{
                    "role":"assistant","content":exact_content
                },"finish_reason":null}]
            }))
            .as_bytes(),
        )
        .unwrap();
    assert_eq!(decoder.kimi_private_retained_lengths(), (7, 0, 0));

    let overflow = decoder.push_batch(
        event(json!({
            "id":"combined-budget","object":"chat.completion.chunk","model":"kimi-k3",
            "choices":[{"index":0,"delta":{"content":"x"},"finish_reason":null}]
        }))
        .as_bytes(),
    );
    assert!(overflow.failure.is_some());
    assert!(overflow.events.is_empty());
    assert_eq!(decoder.kimi_private_retained_lengths(), (7, 0, 0));
}

// private budget은 tool-call object와 배열 구조까지 canonical JSON 크기에 포함하므로
// 같은 tool round가 exact 경계에서는 통과하고 한 바이트 작은 상한에서는 거절됩니다.
#[test]
fn kimi_private_budget_counts_tool_call_structure_exactly() {
    let call = json!({
        "id":"call-1",
        "type":"function",
        "function":{"name":"read_file","arguments":"{}"}
    });
    let chunk = json!({
        "id":"kimi-tool-budget","object":"chat.completion.chunk","model":"kimi-k2.7-code",
        "choices":[{"index":0,"delta":{
            "role":"assistant","reasoning_content":"hidden","tool_calls":[{
                "index":0,"id":"call-1","type":"function",
                "function":{"name":"read_file","arguments":"{}"}
            }]
        },"finish_reason":"tool_calls",
        "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}]
    });
    let exact = serde_json::to_vec(&json!({
        "role":"assistant",
        "reasoning_content":"hidden",
        "content":null,
        "tool_calls":[call]
    }))
    .unwrap()
    .len();
    let mut exact_decoder = ChatCompletionsSseDecoder::new_kimi(
        ResponsesConnectorLimits {
            max_provider_private_bytes: exact,
            ..ResponsesConnectorLimits::default()
        },
        "kimi-k2.7-code".to_owned(),
        true,
    );
    exact_decoder.push(event(chunk.clone()).as_bytes()).unwrap();

    let mut overflow = ChatCompletionsSseDecoder::new_kimi(
        ResponsesConnectorLimits {
            max_provider_private_bytes: exact - 1,
            ..ResponsesConnectorLimits::default()
        },
        "kimi-k2.7-code".to_owned(),
        true,
    );
    assert_eq!(
        overflow.push(event(chunk).as_bytes()).unwrap_err().kind(),
        super::super::ConnectorFailureKind::Limit
    );
}

// response tool identity는 private 저장 단계까지 미루지 않고 Connector가 첫 fragment를
// 보관하기 전에 3..=64 ASCII 이름과 1..=4,096-byte ID를 판별합니다.
#[test]
fn kimi_stream_validates_response_tool_identity_before_retention() {
    for valid in ["abc".to_owned(), format!("a{}", "b".repeat(63))] {
        let mut decoder = ChatCompletionsSseDecoder::new_kimi(
            ResponsesConnectorLimits::default(),
            "kimi-k3".to_owned(),
            true,
        );
        decoder
            .push(
                event(json!({
                    "id":"tool-name","object":"chat.completion.chunk","model":"kimi-k3",
                    "choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
                        "index":0,"id":"call-1","type":"function",
                        "function":{"name":valid,"arguments":""}
                    }]},"finish_reason":null}]
                }))
                .as_bytes(),
            )
            .unwrap();
        assert_eq!(decoder.kimi_private_retained_lengths().2, 1);
    }
    for invalid in [
        "ab".to_owned(),
        format!("a{}", "b".repeat(64)),
        "bad.name".to_owned(),
        "한글도구".to_owned(),
    ] {
        let mut decoder = ChatCompletionsSseDecoder::new_kimi(
            ResponsesConnectorLimits::default(),
            "kimi-k3".to_owned(),
            true,
        );
        assert!(
            decoder
                .push(
                    event(json!({
                        "id":"tool-name","object":"chat.completion.chunk","model":"kimi-k3",
                        "choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
                            "index":0,"id":"call-1","type":"function",
                            "function":{"name":invalid,"arguments":""}
                        }]},"finish_reason":null}]
                    }))
                    .as_bytes(),
                )
                .is_err()
        );
        assert_eq!(decoder.kimi_private_retained_lengths().2, 0);
    }

    for valid_id in ["x".to_owned(), "x".repeat(4 * 1024)] {
        let mut decoder = ChatCompletionsSseDecoder::new_kimi(
            ResponsesConnectorLimits::default(),
            "kimi-k3".to_owned(),
            true,
        );
        decoder
            .push(
                event(json!({
                    "id":"tool-id","object":"chat.completion.chunk","model":"kimi-k3",
                    "choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
                        "index":0,"id":valid_id,"type":"function",
                        "function":{"name":"read_file","arguments":""}
                    }]},"finish_reason":null}]
                }))
                .as_bytes(),
            )
            .unwrap();
        assert_eq!(decoder.kimi_private_retained_lengths().2, 1);
    }
    for invalid_id in [String::new(), "x".repeat(4 * 1024 + 1)] {
        let mut decoder = ChatCompletionsSseDecoder::new_kimi(
            ResponsesConnectorLimits::default(),
            "kimi-k3".to_owned(),
            true,
        );
        assert!(
            decoder
                .push(
                    event(json!({
                        "id":"tool-id","object":"chat.completion.chunk","model":"kimi-k3",
                        "choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
                            "index":0,"id":invalid_id,"type":"function",
                            "function":{"name":"read_file","arguments":""}
                        }]},"finish_reason":null}]
                    }))
                    .as_bytes(),
                )
                .is_err()
        );
        assert_eq!(decoder.kimi_private_retained_lengths().2, 0);
    }
}

// Kimi가 usage를 finish와 같은 choice 또는 뒤의 empty-choice에만 싣는 닫힌 순서를
// 지키지 않으면, 미완료 round의 비용 정보를 final usage로 먼저 채택하지 않습니다.
#[test]
fn kimi_stream_rejects_usage_before_the_finish_reason() {
    for chunk in [
        json!({
            "id":"kimi-1","object":"chat.completion.chunk","model":"kimi-k3",
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2},
            "choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]
        }),
        json!({
            "id":"kimi-1","object":"chat.completion.chunk","model":"kimi-k3",
            "choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null,
                "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}]
        }),
    ] {
        let mut decoder = ChatCompletionsSseDecoder::new_kimi(
            ResponsesConnectorLimits::default(),
            "kimi-k3".to_owned(),
            true,
        );
        let error = decoder.push(event(chunk).as_bytes()).unwrap_err();
        assert!(error.to_string().contains("before the finish"), "{error}");
    }
}
