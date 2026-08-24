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

// duplicated secret-free preflight와 Connector 방어 검사가 drift하지 않도록 모든 exact
// ModelId/alias, K3 effort, Code context form과 주요 one-off 경계를 한 표로 고정합니다.
#[test]
fn complete_kimi_matrix_and_alias_wires_are_table_pinned() {
    for (name, effort) in [
        ("low", ReasoningEffort::Low),
        ("high", ReasoningEffort::High),
        ("max", ReasoningEffort::Max),
    ] {
        let reasoning = format!(r#"{{"effort":"{name}"}}"#);
        let platform = complete(
            "kimi-k3",
            131_073,
            131_072,
            &reasoning,
            "{}",
            "kimi-private-local-plaintext/v1",
        );
        assert!(matches!(
            admit_binding(&platform).unwrap().kind,
            KimiWireKind::PlatformK3 { effort: admitted } if admitted == effort
        ));
        assert_eq!(
            body_for_complete(&platform, Some(effort))["reasoning_effort"],
            name
        );

        for (model, input) in [("k3", 262_144), ("k3", 1_048_576), ("k3-256k", 262_144)] {
            let code = complete_at(
                "https://api.kimi.com/coding/v1",
                model,
                input,
                131_072,
                &reasoning,
                r#"{"thinking":{"type":"enabled","keep":"all"}}"#,
                "kimi-private-local-plaintext/v1",
            );
            assert!(matches!(
                admit_binding(&code).unwrap().kind,
                KimiWireKind::CodeK3 { effort: admitted } if admitted == effort
            ));
            let body = body_for_complete(&code, Some(effort));
            assert_eq!(body["reasoning_effort"], name);
            assert_eq!(body["thinking"], json!({"type":"enabled","keep":"all"}));
            assert!(body["prompt_cache_key"].is_string());
        }
    }

    for model in ["kimi-k2.7-code", "kimi-k2.7-code-highspeed"] {
        let complete = complete(
            model,
            262_144,
            32_768,
            "{}",
            r#"{"thinking":{"type":"enabled","keep":"all"}}"#,
            "kimi-private-local-plaintext/v1",
        );
        assert_eq!(
            admit_binding(&complete).unwrap().kind,
            KimiWireKind::PlatformK27Code
        );
        assert_eq!(
            body_for_complete(&complete, None)["stream_options"]["include_usage"],
            true
        );
    }
    let k26 = complete(
        "kimi-k2.6",
        262_144,
        32_768,
        "{}",
        r#"{"thinking":{"type":"disabled"}}"#,
        "semantic-only/v1",
    );
    assert_eq!(admit_binding(&k26).unwrap().kind, KimiWireKind::PlatformK26);
    assert_eq!(
        body_for_complete(&k26, None)["thinking"]["type"],
        "disabled"
    );

    for model in ["kimi-for-coding", "kimi-for-coding-highspeed"] {
        let complete = complete_at(
            "https://api.kimi.com/coding/v1",
            model,
            262_144,
            32_768,
            "{}",
            r#"{"thinking":{"type":"enabled","keep":"all"}}"#,
            "kimi-private-local-plaintext/v1",
        );
        assert_eq!(
            admit_binding(&complete).unwrap().kind,
            KimiWireKind::CodeK27
        );
        assert!(body_for_complete(&complete, None)["prompt_cache_key"].is_string());
    }

    assert!(CompleteModelBinding::from_durable_json(
        r#"{"provider":"kimi","account":"default","model":"kimi-k3","connector":"kimi-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":131072,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
    ).is_err());

    for invalid in [
        complete(
            "kimi-k3",
            1_048_576,
            131_072,
            r#"{"effort":"medium"}"#,
            "{}",
            "kimi-private-local-plaintext/v1",
        ),
        complete(
            "kimi-k2.6",
            262_144,
            32_768,
            "{}",
            r#"{"thinking":{"type":"disabled"}}"#,
            "kimi-private-local-plaintext/v1",
        ),
        complete_at(
            "https://api.kimi.com/coding/v1",
            "k3-256k",
            1_048_576,
            131_072,
            r#"{"effort":"high"}"#,
            r#"{"thinking":{"type":"enabled","keep":"all"}}"#,
            "kimi-private-local-plaintext/v1",
        ),
        complete_at(
            "https://api.kimi.com/coding/v1",
            "kimi-for-coding",
            262_144,
            32_769,
            "{}",
            r#"{"thinking":{"type":"enabled","keep":"all"}}"#,
            "kimi-private-local-plaintext/v1",
        ),
        complete_at(
            "https://api.moonshot.ai/v1",
            "k3",
            262_144,
            131_072,
            r#"{"effort":"high"}"#,
            r#"{"thinking":{"type":"enabled","keep":"all"}}"#,
            "kimi-private-local-plaintext/v1",
        ),
    ] {
        assert!(admit_binding(&invalid).is_err());
    }
}

// admitted tool policy를 Connector가 끝까지 보존해 no-tools wire 노출을 스스로 막는지 검증합니다.
#[test]
fn connector_enforces_local_tools_and_no_tools_profiles() {
    let tool = FunctionTool::new(
        "read_file",
        "read a file",
        json!({"type":"object","properties":{},"additionalProperties":false}),
    )
    .unwrap();
    let request = |exposure| {
        ModelConnectorRequest::new(
            vec![ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::User,
                content: "hello".to_owned(),
                refusal: None,
            }],
            exposure,
            131_072,
            Some(ReasoningEffort::Max),
        )
        .unwrap()
    };
    let local = k3();
    let local_body = wire_body(
        &request(RequestToolExposure::enabled(vec![tool.clone()])),
        "kimi-k3",
        admit_binding(&local).unwrap(),
    )
    .unwrap();
    assert_eq!(local_body["tool_choice"], "auto");
    assert_eq!(local_body["tools"].as_array().unwrap().len(), 1);

    let no_tools = CompleteModelBinding::from_durable_json(
        r#"{"provider":"kimi","account":"default","model":"kimi-k3","connector":"kimi-chat-completions","base_url":"https://api.moonshot.ai/v1","api_dialect":"kimi-chat-completions","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1048576,"max_output_tokens":131072,"reasoning_parameters":{"effort":"max"},"optional_request_parameters":{},"tool_capability_policy":"no-tools/v1","replay_profile":"kimi-private-local-plaintext/v1"}"#,
    )
    .unwrap();
    let profile = admit_binding(&no_tools).unwrap();
    assert!(
        wire_body(
            &request(RequestToolExposure::enabled(vec![tool])),
            "kimi-k3",
            profile,
        )
        .is_err()
    );
    let body = wire_body(
        &request(RequestToolExposure::disabled()),
        "kimi-k3",
        profile,
    )
    .unwrap();
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
}

// Platform K3 wire는 profile에서 확정한 output/reasoning만 보내고 deprecated sampling 및
// stream_options를 넣지 않아 다른 Chat Completions 의미를 우연히 상속하지 않습니다.
#[test]
fn platform_k3_request_uses_only_the_closed_kimi_wire_fields() {
    let profile = admit_binding(&k3()).unwrap();
    let request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
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
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }]
    };
    let smaller = ModelConnectorRequest::new(
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
        let request = ModelConnectorRequest::new(
            input(),
            RequestToolExposure::disabled(),
            cap,
            Some(ReasoningEffort::Max),
        )
        .unwrap();
        assert!(wire_body(&request, "kimi-k3", profile).is_err());
    }
}

// 1M Code k3는 hard max 이하의 request cap, final usage 요청, preserved-thinking,
// caller의 typed cache hint를 함께 보냅니다. hint가 없거나 endpoint/ModelId가 교차되면
// transport 전에 실패합니다.
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
    let request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        4_096,
        Some(ReasoningEffort::High),
    )
    .unwrap();
    assert!(wire_body(&request, "k3", profile).is_err());

    let session = fixture_session(31);
    let request = request.with_cache_affinity_hint(ModelCacheAffinityHint::for_session(session));
    let body = wire_body(&request, "k3", profile).unwrap();
    assert_eq!(body["max_completion_tokens"], 4_096);
    assert_eq!(body["stream_options"]["include_usage"], true);
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
    let session = fixture_session(32);
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
    let code_request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
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

    let platform_request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
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
    let request = ModelConnectorRequest::new(
        vec![
            ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::Assistant,
                content: "visible".to_owned(),
                refusal: None,
            },
            ModelConnectorInputItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"README.md"}"#.to_owned(),
            },
            ModelConnectorInputItem::ProviderPrivateAssistant {
                envelope: encode_envelope(&private).unwrap(),
            },
            ModelConnectorInputItem::FunctionCallOutput {
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

fn private_tool_round(call_id: &str) -> Vec<ModelConnectorInputItem> {
    vec![
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::Assistant,
            content: String::new(),
            refusal: None,
        },
        ModelConnectorInputItem::FunctionCall {
            call_id: call_id.to_owned(),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"README.md"}"#.to_owned(),
        },
        ModelConnectorInputItem::ProviderPrivateAssistant {
            envelope: encode_envelope(&KimiAssistantMessage::new(
                "hidden",
                None,
                vec![KimiAssistantToolCall::new(
                    call_id,
                    "read_file",
                    r#"{"path":"README.md"}"#,
                )],
            ))
            .unwrap(),
        },
    ]
}

// public Connector request도 managed replay 검증에 기대지 않고, 도구 결과가 앞선 assistant
// 호출과 전역적으로 한 번만 대응하며 모든 호출이 응답된 경우에만 transport 전 body를 만듭니다.
#[test]
fn replay_rejects_orphan_reordered_duplicate_and_unanswered_tool_relationships() {
    let output = |call_id: &str| ModelConnectorInputItem::FunctionCallOutput {
        call_id: call_id.to_owned(),
        output: "contents".to_owned(),
    };
    let mut output_before_call = vec![output("call-1")];
    output_before_call.extend(private_tool_round("call-1"));
    output_before_call.push(output("call-1"));

    let mut duplicate_output = private_tool_round("call-1");
    duplicate_output.extend([output("call-1"), output("call-1")]);

    let mut duplicate_call = private_tool_round("call-1");
    duplicate_call.push(output("call-1"));
    duplicate_call.extend(private_tool_round("call-1"));
    duplicate_call.push(output("call-1"));

    for (input, expected) in [
        (vec![output("orphan")], "no prior matching function call"),
        (output_before_call, "no prior matching function call"),
        (duplicate_output, "duplicate function call output"),
        (duplicate_call, "duplicate function call identity"),
        (
            private_tool_round("call-unanswered"),
            "unanswered function call",
        ),
    ] {
        let request = ModelConnectorRequest::new(
            input,
            RequestToolExposure::disabled(),
            131_072,
            Some(ReasoningEffort::Max),
        )
        .unwrap();
        let error = wire_body(&request, "kimi-k3", admit_binding(&k3()).unwrap()).unwrap_err();
        assert!(error.message().contains(expected), "{error}");
    }
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
        let request = ModelConnectorRequest::new(
            vec![
                ModelConnectorInputItem::Message {
                    role: ModelConnectorInputRole::Assistant,
                    content: String::new(),
                    refusal: None,
                },
                ModelConnectorInputItem::FunctionCall {
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"README.md"}"#.to_owned(),
                },
                ModelConnectorInputItem::ProviderPrivateAssistant {
                    envelope: encode_envelope(&KimiAssistantMessage::new(
                        "hidden",
                        None,
                        vec![private],
                    ))
                    .unwrap(),
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
    let request = ModelConnectorRequest::new(
        vec![
            ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::Assistant,
                content: String::new(),
                refusal: None,
            },
            ModelConnectorInputItem::ProviderPrivateAssistant {
                envelope: yo_core::ProviderPrivateReplayEnvelope::new(
                    "kimi.assistant-message/v1alpha1",
                    br#"{"role":"assistant","reasoning_content":"","content":null}"#.to_vec(),
                )
                .unwrap(),
            },
        ],
        RequestToolExposure::disabled(),
        131_072,
        Some(ReasoningEffort::Max),
    )
    .unwrap();

    assert!(wire_body(&request, "kimi-k3", profile).is_err());
}

// Kimi crate가 private wire의 exact member order와 required/nullable 구분을 소유하고,
// core의 opaque envelope는 정상 payload bytes를 재정렬하지 않습니다.
#[test]
fn private_envelope_preserves_exact_stop_and_tool_payload_bytes() {
    let stop = encode_envelope(&KimiAssistantMessage::new(
        "private reasoning",
        Some("visible".to_owned()),
        Vec::new(),
    ))
    .unwrap();
    assert_eq!(
        stop.payload(),
        br#"{"role":"assistant","reasoning_content":"private reasoning","content":"visible"}"#
    );
    assert!(decode_envelope(&stop).is_ok());

    let tool = encode_envelope(&KimiAssistantMessage::new(
        "private reasoning",
        None,
        vec![KimiAssistantToolCall::new("call-1", "read_file", "{}")],
    ))
    .unwrap();
    assert_eq!(
        tool.payload(),
        br#"{"role":"assistant","reasoning_content":"private reasoning","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}"}}]}"#
    );
    assert!(decode_envelope(&tool).is_ok());
}

// omission, explicit null, empty arrays, duplicate outer/nested members는 서로 다른 wire
// 상태이므로 canonicalization 전에 fail-closed 합니다.
#[test]
fn private_envelope_rejects_presence_and_duplicate_member_ambiguity() {
    for malformed in [
        br#"{"role":"assistant","reasoning_content":"hidden"}"#.as_slice(),
        br#"{"role":"assistant","reasoning_content":"hidden","content":null}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":null}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[]}"#,
        br#"{"role":"user","reasoning_content":"hidden","content":"visible"}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":"visible","unknown":true}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"hosted","function":{"name":"read_file","arguments":"{}"}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"","type":"function","function":{"name":"read_file","arguments":"{}"}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"x","arguments":"{}"}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{"}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}","unknown":true}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}"}},{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}"}}]}"#,
    ] {
        let envelope = yo_core::ProviderPrivateReplayEnvelope::new(
            "kimi.assistant-message/v1alpha1",
            malformed.to_vec(),
        )
        .unwrap();
        assert!(decode_envelope(&envelope).is_err());
    }

    for duplicate in [
        br#"{"role":"assistant","role":"assistant","reasoning_content":"hidden","content":"visible"}"#.as_slice(),
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","name":"write_file","arguments":"{}"}}]}"#,
    ] {
        assert!(
            yo_core::ProviderPrivateReplayEnvelope::new(
                "kimi.assistant-message/v1alpha1",
                duplicate.to_vec(),
            )
            .is_err()
        );
    }

    let wrong_schema = yo_core::ProviderPrivateReplayEnvelope::new(
        "other.assistant/v1",
        br#"{"role":"assistant","reasoning_content":"hidden","content":"visible"}"#.to_vec(),
    )
    .unwrap();
    assert!(decode_envelope(&wrong_schema).is_err());

    let mixed = encode_envelope(&KimiAssistantMessage::new(
        "hidden",
        Some("visible".to_owned()),
        vec![KimiAssistantToolCall::new("call-1", "read_file", "{}")],
    ))
    .unwrap();
    let decoded = decode_envelope(&mixed).unwrap();
    assert_eq!(decoded.content(), Some("visible"));
    assert_eq!(decoded.tool_calls().len(), 1);
}

// 복구한 private payload도 여러 call의 argument 합계가 exact 4 MiB만 허용하는지 검증합니다.
#[test]
fn private_envelope_enforces_the_aggregate_argument_boundary_on_decode() {
    let argument = |bytes: usize| format!("\"{}\"", "x".repeat(bytes - 2));
    let payload = |second_bytes: usize| {
        let first = serde_json::to_string(&argument(2 * 1024 * 1024)).unwrap();
        let second = serde_json::to_string(&argument(second_bytes)).unwrap();
        format!(
            r#"{{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{{"id":"call-1","type":"function","function":{{"name":"read_file","arguments":{first}}}}},{{"id":"call-2","type":"function","function":{{"name":"read_file","arguments":{second}}}}}]}}"#,
        )
        .into_bytes()
    };

    let exact = yo_core::ProviderPrivateReplayEnvelope::new(
        "kimi.assistant-message/v1alpha1",
        payload(2 * 1024 * 1024),
    )
    .unwrap();
    assert!(decode_envelope(&exact).is_ok());

    let overflow = yo_core::ProviderPrivateReplayEnvelope::new(
        "kimi.assistant-message/v1alpha1",
        payload(2 * 1024 * 1024 + 1),
    )
    .unwrap();
    assert!(decode_envelope(&overflow).is_err());
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
        let request = ModelConnectorRequest::new(
            vec![ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::User,
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
        ModelConnectorLimits::default(),
        "kimi-k3".to_owned(),
        true,
    );
    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ModelConnectorEvent::ReasoningDelta { .. }))
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::ProviderPrivateAssistant { envelope, .. }
            if decode_envelope(envelope).is_ok_and(|message|
                message.reasoning_content() == "hidden" && message.content() == Some("visible"))
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
        ModelConnectorLimits::default(),
        "kimi-k2.7-code".to_owned(),
        true,
    );
    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());
    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::ProviderPrivateAssistant { envelope, .. }
            if decode_envelope(envelope).is_ok_and(|message|
                message.reasoning_content() == "hidden"
                    && message.content().is_none()
                    && message.tool_calls().len() == 1
                    && message.tool_calls()[0].arguments() == r#"{"path":"README.md"}"#)
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ModelConnectorEvent::Terminal { .. }))
            .count(),
        1
    );
}

// Kimi가 cached_tokens를 0으로 보고한 경우에도 absent로 접지 않고 정확한 source
// profile을 가진 reported 측정값으로 보존합니다.
#[test]
fn kimi_usage_preserves_reported_zero_cache_reads() {
    let stream = [
        event(json!({
            "id":"kimi-cache","object":"chat.completion.chunk","model":"kimi-k3",
            "choices":[{"index":0,"delta":{"role":"assistant","content":"done"},"finish_reason":"stop",
                "usage":{"prompt_tokens":4,"completion_tokens":3,"total_tokens":7,
                    "cached_tokens":0}}]
        })),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ChatCompletionsSseDecoder::new_kimi(
        ModelConnectorLimits::default(),
        "kimi-k3".to_owned(),
        true,
    );
    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());

    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::Terminal {
            usage: yo_core::ModelConnectorUsage {
                cache_read_input_tokens: yo_core::CacheReadInputTokens::Reported {
                    tokens: 0,
                    source_profile,
                },
                ..
            },
            ..
        } if source_profile.as_str() == "kimi.usage.cached-tokens/v1"
    )));
}

// Kimi의 cached_tokens 부재는 unsupported나 0으로 바꾸지 않고 absent로 남기며,
// 음수·null·prompt 초과 값은 Provider 보고값을 추측해 고치지 않고 거절합니다.
#[test]
fn kimi_usage_distinguishes_absent_cache_reads_and_rejects_invalid_reports() {
    let usage = json!({"prompt_tokens":4,"completion_tokens":3,"total_tokens":7});
    let decoded = super::sse::decode_usage(&usage).unwrap();
    assert!(matches!(
        decoded.cache_read_input_tokens,
        yo_core::CacheReadInputTokens::Absent { ref source_profile }
            if source_profile.as_str() == "kimi.usage.cached-tokens/v1"
    ));

    for cached_tokens in [json!(-1), serde_json::Value::Null, json!(5)] {
        let mut invalid = usage.clone();
        invalid["cached_tokens"] = cached_tokens;
        assert!(super::sse::decode_usage(&invalid).is_err());
    }
}

// 같은 wire chunk에 반복된 usage라도 cached_tokens 부재와 reported 0은 같은 값이
// 아니므로 하나로 합치지 않고 cache telemetry 불일치로 거절합니다.
#[test]
fn kimi_repeated_usage_requires_exact_cache_read_availability() {
    let absent = json!({"prompt_tokens":4,"completion_tokens":3,"total_tokens":7});
    let mut reported_zero = absent.clone();
    reported_zero["cached_tokens"] = json!(0);
    let chunk = event(json!({
        "id":"kimi-cache-mismatch","object":"chat.completion.chunk","model":"kimi-k3",
        "usage": absent,
        "choices":[{"index":0,"delta":{"role":"assistant","content":"done"},
            "finish_reason":"stop","usage":reported_zero}]
    }));
    let mut decoder = ChatCompletionsSseDecoder::new_kimi(
        ModelConnectorLimits::default(),
        "kimi-k3".to_owned(),
        true,
    );

    let error = decoder.push(chunk.as_bytes()).unwrap_err();
    assert!(error.to_string().contains("inconsistent"), "{error}");
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
            ModelConnectorLimits::default(),
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
        ModelConnectorLimits {
            max_provider_private_bytes: exact,
            ..ModelConnectorLimits::default()
        },
        "kimi-k3".to_owned(),
        true,
    );
    exact_decoder.push(event(first.clone()).as_bytes()).unwrap();

    let mut overflow = ChatCompletionsSseDecoder::new_kimi(
        ModelConnectorLimits {
            max_provider_private_bytes: exact - 1,
            ..ModelConnectorLimits::default()
        },
        "kimi-k3".to_owned(),
        true,
    );
    let error = overflow.push(event(first).as_bytes()).unwrap_err();
    assert_eq!(error.kind(), yo_core::ConnectorFailureKind::Limit);
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
        kimi_replay_round_item_lengths(true, true, json_bytes(exact_content), 0, &[]).unwrap();
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
    let overflow_lengths = kimi_replay_round_item_lengths(
        true,
        true,
        json_bytes(&format!("{exact_content}x")),
        0,
        &[],
    )
    .unwrap();
    assert!(!replay_budget.accepts_item_lengths(&overflow_lengths));

    let mut decoder = ChatCompletionsSseDecoder::new_kimi_with_replay_budget(
        ModelConnectorLimits::default(),
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

// Kimi의 증분 byte 공식은 stop/tool-call과 escaped UTF-8 값 모두에서 concrete replay
// items를 세는 core canonical encoder와 같은 길이를 계산해야 조기 거절 경계가 drift하지 않습니다.
#[test]
fn kimi_incremental_round_sizes_match_the_canonical_replay_encoder() {
    for (content, reasoning, calls) in [
        (Some("line\n한글"), "hidden\\reason", Vec::new()),
        (
            None,
            "tool reasoning",
            vec![KimiAssistantToolCall::new(
                "call-1",
                "read_file",
                r#"{"path":"a\nb"}"#,
            )],
        ),
        (
            Some("partial"),
            "",
            vec![
                KimiAssistantToolCall::new("call-1", "read_file", "{}"),
                KimiAssistantToolCall::new("call-2", "write_file", r#"{"text":"\""}"#),
            ],
        ),
    ] {
        let json_bytes = |value: &str| serde_json::to_string(value).unwrap().len() - 2;
        let sizes = calls
            .iter()
            .map(|call| crate::private_replay::KimiReplayToolCallSize {
                id_json_bytes: json_bytes(call.id()),
                name_json_bytes: json_bytes(call.name()),
                arguments_json_bytes: json_bytes(call.arguments()),
            })
            .collect::<Vec<_>>();
        let incremental_lengths = kimi_replay_round_item_lengths(
            true,
            content.is_some(),
            content.map_or(0, json_bytes),
            json_bytes(reasoning),
            &sizes,
        )
        .unwrap();
        let message =
            KimiAssistantMessage::new(reasoning, content.map(str::to_owned), calls.clone());
        let mut round_items = vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: content.unwrap_or_default().to_owned(),
            refusal: None,
        }];
        round_items.extend(calls.iter().map(|call| ModelReplayItem::FunctionCall {
            call_id: call.id().to_owned(),
            name: call.name().to_owned(),
            arguments: call.arguments().to_owned(),
        }));
        round_items.push(ModelReplayItem::ProviderPrivateAssistant {
            envelope: encode_envelope(&message).unwrap(),
        });
        let contract = ModelReplayContract::new("system", Vec::new());
        let prefix = ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "prefix".to_owned(),
            refusal: None,
        };
        let prefix_budget =
            ModelReplayDelta::replay_budget(Some(&contract), std::iter::once(&prefix)).unwrap();
        let canonical_budget = ModelReplayDelta::replay_budget(
            Some(&contract),
            std::iter::once(&prefix).chain(round_items.iter()),
        )
        .unwrap();

        assert_eq!(
            prefix_budget.encoded_len_with_item_lengths(&incremental_lengths),
            canonical_budget.encoded_len_with_item_lengths(&[]),
        );
    }
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
        ModelConnectorLimits {
            max_provider_private_bytes: exact,
            ..ModelConnectorLimits::default()
        },
        "kimi-k2.7-code".to_owned(),
        true,
    );
    exact_decoder.push(event(chunk.clone()).as_bytes()).unwrap();

    let mut overflow = ChatCompletionsSseDecoder::new_kimi(
        ModelConnectorLimits {
            max_provider_private_bytes: exact - 1,
            ..ModelConnectorLimits::default()
        },
        "kimi-k2.7-code".to_owned(),
        true,
    );
    assert_eq!(
        overflow.push(event(chunk).as_bytes()).unwrap_err().kind(),
        yo_core::ConnectorFailureKind::Limit
    );
}

// caller limit은 Kimi의 고정 1,024-call 상한을 높일 권한이 아니며, 첫 초과 call은
// decoder가 보존하기 전에 실패해야 합니다.
#[test]
fn kimi_stream_clamps_raised_tool_call_limit_before_retention() {
    let tool_calls = (0..=1_024)
        .map(|index| {
            json!({
                "index": index,
                "id": format!("call-{index}"),
                "type": "function",
                "function": {"name": "read_file", "arguments": "{}"},
            })
        })
        .collect::<Vec<_>>();
    let mut decoder = ChatCompletionsSseDecoder::new_kimi(
        ModelConnectorLimits {
            max_output_items: usize::MAX,
            ..ModelConnectorLimits::default()
        },
        "kimi-k3".to_owned(),
        true,
    );
    let batch = decoder.push_batch(
        event(json!({
            "id":"kimi-call-bound","object":"chat.completion.chunk","model":"kimi-k3",
            "choices":[{"index":0,"delta":{"role":"assistant","tool_calls":tool_calls},
                "finish_reason":null}]
        }))
        .as_bytes(),
    );
    assert_eq!(
        batch.failure.as_ref().map(yo_core::ConnectorError::kind),
        Some(yo_core::ConnectorFailureKind::Limit)
    );
    assert_eq!(decoder.kimi_private_retained_lengths().2, 1_024);
}

// 높여 전달한 caller argument limit도 계약된 합계 4 MiB로 제한되어야 하며, +1 byte는
// 보존 중인 call에 추가되기 전에 실패해야 합니다.
#[test]
fn kimi_stream_clamps_raised_argument_limit_before_retention() {
    let mut decoder = ChatCompletionsSseDecoder::new_kimi(
        ModelConnectorLimits {
            max_function_argument_bytes: usize::MAX,
            max_provider_private_bytes: usize::MAX,
            ..ModelConnectorLimits::default()
        },
        "kimi-k3".to_owned(),
        true,
    );
    decoder
        .push(
            event(json!({
                "id":"kimi-argument-bound","object":"chat.completion.chunk","model":"kimi-k3",
                "choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
                    "index":0,"id":"call-1","type":"function",
                    "function":{"name":"read_file","arguments":""}
                }]},"finish_reason":null}]
            }))
            .as_bytes(),
        )
        .unwrap();

    let fragment = "x".repeat(512 * 1024);
    for _ in 0..8 {
        decoder
            .push(
                event(json!({
                    "id":"kimi-argument-bound","object":"chat.completion.chunk","model":"kimi-k3",
                    "choices":[{"index":0,"delta":{"tool_calls":[{
                        "index":0,"function":{"arguments":fragment}
                    }]},"finish_reason":null}]
                }))
                .as_bytes(),
            )
            .unwrap();
    }
    assert_eq!(decoder.kimi_retained_argument_bytes(), 4 * 1024 * 1024);
    let batch = decoder.push_batch(
        event(json!({
            "id":"kimi-argument-bound","object":"chat.completion.chunk","model":"kimi-k3",
            "choices":[{"index":0,"delta":{"tool_calls":[{
                "index":0,"function":{"arguments":"x"}
            }]},"finish_reason":null}]
        }))
        .as_bytes(),
    );
    assert_eq!(
        batch.failure.as_ref().map(yo_core::ConnectorError::kind),
        Some(yo_core::ConnectorFailureKind::Limit)
    );
    assert_eq!(decoder.kimi_retained_argument_bytes(), 4 * 1024 * 1024);
}

// response tool identity는 private 저장 단계까지 미루지 않고 Connector가 첫 fragment를
// 보관하기 전에 3..=64 ASCII 이름과 1..=4,096-byte ID를 판별합니다.
#[test]
fn kimi_stream_validates_response_tool_identity_before_retention() {
    for valid in ["abc".to_owned(), format!("a{}", "b".repeat(63))] {
        let mut decoder = ChatCompletionsSseDecoder::new_kimi(
            ModelConnectorLimits::default(),
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
            ModelConnectorLimits::default(),
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
            ModelConnectorLimits::default(),
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
            ModelConnectorLimits::default(),
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
            ModelConnectorLimits::default(),
            "kimi-k3".to_owned(),
            true,
        );
        let error = decoder.push(event(chunk).as_bytes()).unwrap_err();
        assert!(error.to_string().contains("before the finish"), "{error}");
    }
}

// `[DONE]`이 terminal evidence보다 먼저 오면 성공시키지 않되, finish와 usage가 모두
// 없는 경우와 finish만 있고 usage가 없는 경우를 구분해 다음 wire 진단을 좁힙니다.
#[test]
fn kimi_done_failure_identifies_the_missing_terminal_evidence() {
    let mut empty = ChatCompletionsSseDecoder::new_kimi(
        ModelConnectorLimits::default(),
        "kimi-k3".to_owned(),
        true,
    );
    let error = empty.push(b"data: [DONE]\n\n").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("before finish reason and final usage"),
        "{error}"
    );

    let stream = [
        event(json!({
            "id":"kimi-no-usage","object":"chat.completion.chunk","model":"kimi-k3",
            "choices":[{"index":0,"delta":{"role":"assistant","content":"done"},
                "finish_reason":"stop"}]
        })),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut missing_usage = ChatCompletionsSseDecoder::new_kimi(
        ModelConnectorLimits::default(),
        "kimi-k3".to_owned(),
        true,
    );
    let error = missing_usage.push(stream.as_bytes()).unwrap_err();
    assert!(error.to_string().contains("before final usage"), "{error}");
    assert!(!error.to_string().contains("finish reason"), "{error}");
}
