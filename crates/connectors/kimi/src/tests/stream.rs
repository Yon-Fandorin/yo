use super::*;

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
