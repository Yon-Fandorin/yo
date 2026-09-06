use super::*;

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
