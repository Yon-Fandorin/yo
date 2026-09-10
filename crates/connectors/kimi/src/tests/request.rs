use base64::{Engine as _, engine::general_purpose::STANDARD};
use yo_core::{InputImageSnapshot, ModelConnector, ModelInputPart};

use super::*;

// 동일한 secret-free preflight와 Connector 방어 검사가 drift하지 않도록 모든 exact
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
        assert_admitted_semantics(&platform, Some(effort), true);
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
            assert_admitted_semantics(&code, Some(effort), true);
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
        assert_admitted_semantics(&complete, None, true);
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
    assert_admitted_semantics(&k26, None, false);
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
        assert_admitted_semantics(&complete, None, true);
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
        assert!(crate::admit_complete_binding(&invalid).is_err());
        assert!(
            crate::KimiChatCompletionsConnector::new(
                &invalid,
                yo_core::ApiCredential::new("fixture-key").unwrap(),
                ModelConnectorLimits::default(),
            )
            .is_err()
        );
    }
}

fn assert_admitted_semantics(
    complete: &CompleteModelBinding,
    effort: Option<ReasoningEffort>,
    private: bool,
) {
    let admitted = crate::admit_complete_binding(complete).unwrap();
    assert_eq!(admitted.profile().reasoning_effort(), effort);
    assert_eq!(
        admitted.profile().tool_policy(),
        yo_core::AdmittedToolPolicy::LocalTools
    );
    assert_eq!(
        admitted.replay_profile(),
        if private {
            yo_core::AdmittedReplayProfile::ProviderPrivateLocalPlaintext
        } else {
            yo_core::AdmittedReplayProfile::SemanticOnly
        }
    );
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
    let admitted = crate::admit_complete_binding(&no_tools).unwrap();
    assert_eq!(
        admitted.profile().tool_policy(),
        yo_core::AdmittedToolPolicy::NoTools
    );
    assert_eq!(
        admitted.profile().reasoning_effort(),
        Some(ReasoningEffort::Max)
    );
    assert_eq!(
        admitted.replay_profile(),
        yo_core::AdmittedReplayProfile::ProviderPrivateLocalPlaintext
    );
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

const IMAGE_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

fn image_part() -> ModelInputPart {
    ModelInputPart::Image {
        snapshot: InputImageSnapshot::new(1, 1, STANDARD.decode(IMAGE_PNG).unwrap()).unwrap(),
    }
}

fn image_complete(model: &str, enabled: bool) -> CompleteModelBinding {
    let (output, reasoning) = if model.starts_with("kimi-for-") {
        (32_768, json!({}))
    } else {
        (131_072, json!({"effort": "high"}))
    };
    let mut complete = json!({
        "provider": "kimi", "account": "default", "model": model,
        "connector": "kimi-chat-completions", "base_url": "https://api.kimi.com/coding/v1",
        "api_dialect": "kimi-chat-completions", "tokenizer_profile": "utf8-bytes/v1",
        "input_token_limit": 262_144, "max_output_tokens": output,
        "reasoning_parameters": reasoning,
        "optional_request_parameters": {"thinking": {"type": "enabled", "keep": "all"}},
        "tool_capability_policy": "local-tools/v1", "replay_profile": "kimi-private-local-plaintext/v1",
    });
    if enabled {
        complete["image_input_profile"] = json!(yo_core::KIMI_CODE_IMAGE_INPUT_PROFILE);
    }
    CompleteModelBinding::from_durable_json(&complete.to_string()).unwrap()
}

fn image_request(input: Vec<ModelConnectorInputItem>, model: &str) -> ModelConnectorRequest {
    ModelConnectorRequest::new(
        input,
        RequestToolExposure::disabled(),
        4096,
        (!model.starts_with("kimi-for-")).then_some(ReasoningEffort::High),
    )
    .unwrap()
    .with_cache_affinity_hint(ModelCacheAffinityHint::for_session(fixture_session(91)))
}

// 네 Code 모델은 승인된 프로필 아래 텍스트/이미지 순서와 반복 이미지를 그대로 전송한다.
#[test]
fn reviewed_code_image_wire_preserves_order_and_exact_canonical_png() {
    for model in [
        "k3",
        "k3-256k",
        "kimi-for-coding",
        "kimi-for-coding-highspeed",
    ] {
        let complete = image_complete(model, true);
        let request = image_request(
            vec![
                ModelConnectorInputItem::Message {
                    role: ModelConnectorInputRole::User,
                    content: "plain user".into(),
                    refusal: None,
                },
                ModelConnectorInputItem::MultimodalUser {
                    parts: vec![
                        ModelInputPart::Text {
                            text: "before".into(),
                        },
                        image_part(),
                        ModelInputPart::Text {
                            text: "between".into(),
                        },
                        image_part(),
                        ModelInputPart::Text {
                            text: "after\nskill trailer".into(),
                        },
                    ],
                },
            ],
            model,
        );
        let body = wire_body(&request, model, admit_binding(&complete).unwrap()).unwrap();
        let url = format!("data:image/png;base64,{IMAGE_PNG}");
        assert_eq!(
            body["messages"],
            json!([
                {"role": "user", "content": "plain user"},
                {"role": "user", "content": [
                    {"type": "text", "text": "before"},
                    {"type": "image_url", "image_url": {"url": url}},
                    {"type": "text", "text": "between"},
                    {"type": "image_url", "image_url": {"url": url}},
                    {"type": "text", "text": "after\nskill trailer"},
                ]},
            ])
        );
        assert_eq!(body["max_completion_tokens"], 4096);
        assert_eq!(body["thinking"], json!({"type": "enabled", "keep": "all"}));
    }
}

// 프로필이 없는 Code 바인딩과 기존 Platform 바인딩은 이미지 wire를 만들 수 없다.
#[test]
fn image_input_without_reviewed_profile_is_rejected_before_projection() {
    for complete in [
        image_complete("k3", false),
        image_complete("kimi-for-coding", false),
        k3(),
    ] {
        let model = complete.binding().model_id().as_str();
        let request = image_request(
            vec![ModelConnectorInputItem::MultimodalUser {
                parts: vec![image_part()],
            }],
            model,
        );
        let profile = admit_binding(&complete).unwrap();
        for result in [
            wire_body(&request, model, profile),
            crate::request::tokenization_body(&request, model, profile),
        ] {
            let error = result.unwrap_err();
            assert_eq!(error.kind(), yo_core::ConnectorFailureKind::Configuration);
            assert!(error.message().contains("reviewed Code image profile"));
        }
    }
}

// 토큰화는 실제 이미지 URL만 비우고 같은 문자열의 텍스트·도구·비공개 replay는 보존한다.
#[test]
fn tokenization_changes_only_typed_images_after_private_and_tool_projection() {
    let complete = image_complete("k3", true);
    let url = format!("data:image/png;base64,{IMAGE_PNG}");
    let arguments = json!({"value": url}).to_string();
    let private = KimiAssistantMessage::new(
        &url,
        Some(url.clone()),
        vec![KimiAssistantToolCall::new(
            "call-image",
            "inspect_text",
            &arguments,
        )],
    );
    let input = vec![
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::System,
            content: url.clone(),
            refusal: None,
        },
        ModelConnectorInputItem::MultimodalUser {
            parts: vec![ModelInputPart::Text { text: url.clone() }, image_part()],
        },
        ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::Assistant,
            content: url.clone(),
            refusal: None,
        },
        ModelConnectorInputItem::FunctionCall {
            call_id: "call-image".into(),
            name: "inspect_text".into(),
            arguments,
        },
        ModelConnectorInputItem::ProviderPrivateAssistant {
            envelope: encode_envelope(&private).unwrap(),
        },
        ModelConnectorInputItem::FunctionCallOutput {
            call_id: "call-image".into(),
            output: url.clone(),
        },
        ModelConnectorInputItem::MultimodalUser {
            parts: vec![image_part(), ModelInputPart::Text { text: url.clone() }],
        },
    ];
    let tool = FunctionTool::new(
        "inspect_text",
        &url,
        json!({
            "type": "object", "properties": {"value": {"type": "string"}},
            "required": ["value"], "additionalProperties": false,
        }),
    )
    .unwrap();
    let request = ModelConnectorRequest::new(
        input,
        RequestToolExposure::enabled(vec![tool]),
        4096,
        Some(ReasoningEffort::High),
    )
    .unwrap()
    .with_cache_affinity_hint(ModelCacheAffinityHint::for_session(fixture_session(92)));
    let connector = crate::KimiChatCompletionsConnector::new(
        &complete,
        yo_core::ApiCredential::new("synthetic-test-credential").unwrap(),
        ModelConnectorLimits::default(),
    )
    .unwrap();
    let mut expected = wire_body(&request, "k3", admit_binding(&complete).unwrap()).unwrap();
    assert_eq!(expected["messages"].as_array().unwrap().len(), 5);
    assert_eq!(expected["messages"][2]["reasoning_content"], url);
    expected["messages"][1]["content"][1]["image_url"]["url"] = json!("");
    expected["messages"][4]["content"][0]["image_url"]["url"] = json!("");
    let actual = connector.tokenization_payload(&request).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["messages"][0]["content"], url);
    assert_eq!(actual["messages"][1]["content"][0]["text"], url);
    assert_eq!(actual["messages"][3]["content"], url);
    assert_eq!(actual["tools"][0]["function"]["description"], url);
}

// 이미지 없는 요청은 image 프로필의 유무와 관계없이 기존 토큰화·전송 값을 유지한다.
#[test]
fn text_only_tokenization_retains_exact_transport_request() {
    for enabled in [false, true] {
        let complete = image_complete("k3", enabled);
        let request = image_request(
            vec![ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::User,
                content: format!("data:image/png;base64,{IMAGE_PNG}"),
                refusal: None,
            }],
            "k3",
        );
        let profile = admit_binding(&complete).unwrap();
        assert_eq!(
            crate::request::tokenization_body(&request, "k3", profile).unwrap(),
            wire_body(&request, "k3", profile).unwrap(),
        );
    }
}

// 입력별 이미지 한도를 이미 통과한 과거 메시지에는 대화 전체 16개 한도를 추가하지 않는다.
#[test]
fn ordinary_image_history_is_not_limited_to_one_input_image_count() {
    let complete = image_complete("k3", true);
    let request = image_request(
        (0..17)
            .map(|_| ModelConnectorInputItem::MultimodalUser {
                parts: vec![image_part()],
            })
            .collect(),
        "k3",
    );
    let profile = admit_binding(&complete).unwrap();
    let wire = wire_body(&request, "k3", profile).unwrap();
    let counter = crate::request::tokenization_body(&request, "k3", profile).unwrap();
    assert_eq!(wire["messages"].as_array().unwrap().len(), 17);
    assert_eq!(counter["messages"].as_array().unwrap().len(), 17);
    for message in counter["messages"].as_array().unwrap() {
        assert_eq!(
            message["content"][0],
            json!({"type": "image_url", "image_url": {"url": ""}})
        );
    }
}
