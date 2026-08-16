use super::*;

// Kimi decoder의 증분 byte 공식은 stop/tool-call과 escaped UTF-8 값 모두에서 최종
// canonical ModelReplayDelta encoder와 정확히 같은 길이를 계산해야 조기 거절이 안전합니다.
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
            .map(|call| KimiReplayToolCallSize {
                id_json_bytes: json_bytes(call.id()),
                name_json_bytes: json_bytes(call.name()),
                arguments_json_bytes: json_bytes(call.arguments()),
            })
            .collect::<Vec<_>>();
        let lengths = kimi_replay_round_item_lengths(
            true,
            content.is_some(),
            content.map_or(0, json_bytes),
            json_bytes(reasoning),
            &sizes,
        )
        .unwrap();
        let mut items = vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: content.unwrap_or_default().to_owned(),
            refusal: None,
        }];
        items.extend(calls.iter().map(|call| ModelReplayItem::FunctionCall {
            call_id: call.id().to_owned(),
            name: call.name().to_owned(),
            arguments: call.arguments().to_owned(),
        }));
        items.push(ModelReplayItem::ProviderPrivateAssistant {
            schema: "kimi.assistant-message/v1alpha1".to_owned(),
            message: KimiAssistantMessage::new(reasoning, content.map(str::to_owned), calls),
        });
        let contract = ModelReplayContract::new("system", Vec::new());
        let prefix = ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "prefix".to_owned(),
            refusal: None,
        };
        let budget =
            ModelReplayDelta::replay_budget(Some(&contract), std::iter::once(&prefix)).unwrap();

        assert_eq!(
            budget.encoded_len_with_item_lengths(&lengths),
            ModelReplayDelta::prospective_encoded_len(
                Some(&contract),
                std::iter::once(&prefix).chain(items.iter()),
            )
        );
    }
}

// 리플레이 함수 결과는 앞선 정확한 호출과 한 번만 짝지어져야 한다.
#[test]
fn model_replay_rejects_missing_and_duplicate_function_relationships() {
    let mut replay = ModelReplay::default();
    let missing = ModelReplayDelta::new(
        None,
        vec![ModelReplayItem::FunctionCallOutput {
            call_id: "call-1".to_owned(),
            output: "missing".to_owned(),
        }],
    );
    assert!(replay.apply(&missing).is_err());

    let duplicate = ModelReplayDelta::new(
        None,
        vec![
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: "{}".to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "one".to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "two".to_owned(),
            },
        ],
    );
    assert!(replay.apply(&duplicate).is_err());
}

// 유효한 호출과 결과는 원래 인자·출력 바이트를 바꾸지 않고 누적된다.
#[test]
fn model_replay_preserves_one_exact_function_relationship() {
    let delta = ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{ "path": "README.md" }"#.to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "contents".to_owned(),
            },
        ],
    );
    let mut replay = ModelReplay::default();
    replay.apply(&delta).unwrap();
    assert_eq!(replay.items(), delta.items());
}

// visible refusal은 assistant가 낸 관찰에만 의미가 있으므로 system·developer·user 역할에
// 붙은 replay는 저장이나 다음 dialect 직렬화 전에 공통 증거 경계에서 거부한다.
#[test]
fn model_replay_rejects_refusal_on_non_assistant_messages() {
    for role in [
        ModelReplayRole::System,
        ModelReplayRole::Developer,
        ModelReplayRole::User,
    ] {
        let delta = ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            vec![ModelReplayItem::Message {
                role,
                content: String::new(),
                refusal: Some("declined".to_owned()),
            }],
        );

        assert!(!delta.is_valid());
    }
}

// replay contract·delta·전체 prefix가 각 바이트 상한을 넘으면 저장 전에 거부하는지 검증합니다.
#[test]
fn model_replay_enforces_contract_delta_and_prefix_byte_bounds() {
    let oversized_contract =
        ModelReplayContract::new("x".repeat(MAX_REPLAY_CONTRACT_BYTES), Vec::new());
    assert!(!oversized_contract.is_valid());

    let oversized_delta = ModelReplayDelta::new(
        None,
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "x".repeat(MAX_REPLAY_DELTA_BYTES),
            refusal: None,
        }],
    );
    assert!(!oversized_delta.is_valid());

    let body = "x".repeat(13 * 1024 * 1024);
    let mut replay = ModelReplay::default();
    for index in 0..4 {
        replay
            .apply(&ModelReplayDelta::new(
                (index == 0).then(|| ModelReplayContract::new("system", Vec::new())),
                vec![ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: body.clone(),
                    refusal: None,
                }],
            ))
            .unwrap();
    }
    let error = replay
        .apply(&ModelReplayDelta::new(
            None,
            vec![ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: body,
                refusal: None,
            }],
        ))
        .unwrap_err();
    assert!(error.contains("prefix byte limit"));
}

// contract만 있고 semantic item이 없는 delta는 완료 Turn의 replay 증거가 될 수 없음을 검증합니다.
#[test]
fn model_replay_delta_requires_at_least_one_semantic_item() {
    assert!(
        !ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            Vec::new(),
        )
        .is_valid()
    );
}

// replay contract은 chain의 첫 delta에 정확히 한 번만 나타나야 하며 누락·지연 선언을 거부하는지
// 검증합니다.
#[test]
fn model_replay_contract_is_required_on_the_first_delta_only() {
    let message = || ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: "answer".to_owned(),
        refusal: None,
    };
    let mut replay = ModelReplay::default();
    assert!(
        replay
            .apply(&ModelReplayDelta::new(None, vec![message()]))
            .unwrap_err()
            .contains("first")
    );

    replay
        .apply(&ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            vec![message()],
        ))
        .unwrap();
    assert!(
        replay
            .apply(&ModelReplayDelta::new(
                Some(ModelReplayContract::new("late", Vec::new())),
                vec![message()],
            ))
            .unwrap_err()
            .contains("more than once")
    );
}

// Kimi provider-private reasoning과 model-visible 인자는 public Debug 경계를 거쳐도
// 길이와 개수만 남고 원문은 진단 문자열에 노출되지 않습니다.
#[test]
fn kimi_private_debug_is_redacted_through_public_enclosing_types() {
    let reasoning = "private-reasoning-sentinel";
    let arguments = r#"{"private":"argument-sentinel"}"#;
    let message = KimiAssistantMessage::new(
        reasoning,
        None,
        vec![KimiAssistantToolCall::new("call-1", "read_file", arguments)],
    );
    let event = crate::ResponsesEvent::ProviderPrivateAssistant {
        output_index: 1,
        schema: "kimi.assistant-message/v1alpha1".to_owned(),
        message: message.clone(),
    };
    let replay = ModelReplayItem::ProviderPrivateAssistant {
        schema: "kimi.assistant-message/v1alpha1".to_owned(),
        message: message.clone(),
    };

    for rendered in [
        format!("{message:?}"),
        format!("{:?}", message.tool_calls()[0]),
        format!("{event:?}"),
        format!("{replay:?}"),
    ] {
        assert!(!rendered.contains(reasoning), "{rendered}");
        assert!(!rendered.contains("argument-sentinel"), "{rendered}");
        assert!(rendered.contains("reasoning_bytes") || rendered.contains("argument_bytes"));
    }
}

// tool call이 없는 stop형 private assistant는 content 문자열을 반드시 보존해야 하므로
// null content를 빈 visible message와 같은 것으로 축약하지 않습니다.
#[test]
fn kimi_private_stop_requires_present_string_content() {
    let delta = ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: String::new(),
                refusal: None,
            },
            ModelReplayItem::ProviderPrivateAssistant {
                schema: "kimi.assistant-message/v1alpha1".to_owned(),
                message: KimiAssistantMessage::new("", None, Vec::new()),
            },
        ],
    );

    assert!(!delta.is_valid());
    assert!(delta.validate().is_err());
}
