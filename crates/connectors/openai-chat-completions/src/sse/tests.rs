use serde_json::json;

use super::*;

fn event(value: Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(&value).unwrap())
}

fn final_usage(id: &str) -> String {
    event(json!({
        "id": id,
        "choices": [],
        "usage": {
            "prompt_tokens": 4,
            "completion_tokens": 3,
            "total_tokens": 7,
            "completion_tokens_details": {"reasoning_tokens": 1}
        }
    }))
}

// refusal delta와 finish·usage·DONE 순서를 모두 만족한 stream만 resumable terminal을 냅니다.
#[test]
fn decodes_visible_refusal_and_exact_terminal_sequence() {
    let stream = [
        event(json!({
            "id":"chat-1",
            "choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}],
            "usage":null
        })),
        event(json!({
            "id":"chat-1",
            "choices":[{"index":0,"delta":{"refusal":"declined"},"finish_reason":null}]
        })),
        event(json!({
            "id":"chat-1",
            "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]
        })),
        final_usage("chat-1"),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());
    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::RefusalDelta { delta, .. } if delta == "declined"
    )));
    assert!(matches!(
        events.last(),
        Some(ModelConnectorEvent::Terminal {
            status: ModelConnectorTerminal::Completed,
            ..
        })
    ));
}

// role-only delta 뒤 바로 stop이 와도 빈 assistant 응답으로 완료하고 정확한 terminal 순서를
// 허용합니다.
#[test]
fn accepts_a_role_only_empty_stop_completion() {
    let stream = [
        event(json!({
            "id":"chat-empty",
            "choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]
        })),
        event(json!({
            "id":"chat-empty",
            "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]
        })),
        final_usage("chat-empty"),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());

    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());

    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::MessageDone {
            output_index: 0,
            ..
        }
    )));
    assert!(matches!(
        events.last(),
        Some(ModelConnectorEvent::Terminal {
            status: ModelConnectorTerminal::Completed,
            ..
        })
    ));
}

// 한 chunk의 뒤 event가 깨져도 앞의 완전한 관찰은 failure와 함께 반환되어 먼저 전달됩니다.
#[test]
fn preserves_completed_events_before_a_later_same_chunk_failure() {
    let stream = [
        event(json!({
            "id":"chat-partial",
            "choices":[{"index":0,"delta":{"content":"visible"},"finish_reason":null}]
        })),
        "data: {not-json}\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());

    let batch = decoder.push_batch(stream.as_bytes());

    assert!(matches!(
        batch.failure,
        Some(ref failure) if failure.kind() == ConnectorFailureKind::Protocol
    ));
    assert!(batch.events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::TextDelta { delta, .. } if delta == "visible"
    )));
}

// visible content와 여러 fragment의 tool call을 한 round에서 각각 손실 없이 복원합니다.
#[test]
fn decodes_mixed_content_and_tool_calls_without_losing_the_message() {
    let stream = [
        event(json!({
            "id":"chat-2",
            "choices":[{"index":0,"delta":{"content":"checking","tool_calls":[{
                "index":0,"id":"call-1","type":"function",
                "function":{"name":"read_file","arguments":"{\"path\":"}
            }]},"finish_reason":null}]
        })),
        event(json!({
            "id":"chat-2",
            "choices":[{"index":0,"delta":{"tool_calls":[{
                "index":0,"function":{"arguments":"\"README.md\"}"}
            }]},"finish_reason":"tool_calls"}]
        })),
        final_usage("chat-2"),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    let events = decoder.push(stream.as_bytes()).unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ModelConnectorEvent::MessageDone { .. }))
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::FunctionCallDone { arguments, .. }
            if arguments == r#"{"path":"README.md"}"#
    )));
    assert!(decoder.finish().is_ok());
}

// 고정된 call ID 뒤의 명시적 빈 ID는 omission처럼 허용하고 argument bytes를 그대로 잇습니다.
#[test]
fn treats_an_empty_repeated_tool_call_id_as_omission() {
    let stream = [
        event(json!({
            "id":"chat-empty-repeat",
            "choices":[{"index":0,"delta":{"tool_calls":[{
                "index":0,"id":"call-stable","type":"function",
                "function":{"name":"read_files","arguments":"{\"files\":"}
            }]},"finish_reason":null}]
        })),
        event(json!({
            "id":"chat-empty-repeat",
            "choices":[{"index":0,"delta":{"tool_calls":[{
                "index":0,"id":"","type":"function",
                "function":{"arguments":"[]}\n"}
            }]},"finish_reason":"tool_calls"}]
        })),
        final_usage("chat-empty-repeat"),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());

    let events = decoder.push(stream.as_bytes()).unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::FunctionCallDone { call_id, arguments, .. }
            if call_id == "call-stable" && arguments == "{\"files\":[]}\n"
    )));
    assert!(decoder.finish().is_ok());
}

// 빈 최초 ID와 고정 뒤 변경된 비어 있지 않은 ID는 모두 call identity를 만들거나 바꿀 수 없습니다.
#[test]
fn rejects_empty_initial_and_changed_repeated_tool_call_ids() {
    let mut empty_initial = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    let empty_error = empty_initial
        .push(
            event(json!({
                "id":"chat-empty-initial",
                "choices":[{"index":0,"delta":{"tool_calls":[{
                    "index":0,"id":"","type":"function",
                    "function":{"name":"read_files","arguments":"{}"}
                }]},"finish_reason":null}]
            }))
            .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(empty_error.kind(), ConnectorFailureKind::Protocol);

    let mut changed = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    changed
        .push(
            event(json!({
                "id":"chat-changed-repeat",
                "choices":[{"index":0,"delta":{"tool_calls":[{
                    "index":0,"id":"call-one","type":"function",
                    "function":{"name":"read_files","arguments":"{"}
                }]},"finish_reason":null}]
            }))
            .as_bytes(),
        )
        .unwrap();
    let changed_error = changed
        .push(
            event(json!({
                "id":"chat-changed-repeat",
                "choices":[{"index":0,"delta":{"tool_calls":[{
                    "index":0,"id":"call-two","function":{"arguments":"}"}
                }]},"finish_reason":null}]
            }))
            .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(changed_error.kind(), ConnectorFailureKind::Protocol);
}

// response ID 변경, final usage 누락, stream 절단은 정상 종료로 승격하지 않습니다.
#[test]
fn rejects_missing_usage_changed_ids_and_truncated_streams() {
    let mut changed = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    changed
        .push(
            event(json!({
                "id":"one",
                "choices":[{"index":0,"delta":{"content":"a"},"finish_reason":null}]
            }))
            .as_bytes(),
        )
        .unwrap();
    assert!(
        changed
            .push(
                event(json!({
                    "id":"two",
                    "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]
                }))
                .as_bytes()
            )
            .is_err()
    );

    let mut truncated = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    truncated
        .push(
            event(json!({
                "id":"one",
                "choices":[{"index":0,"delta":{"content":"a"},"finish_reason":"length"}]
            }))
            .as_bytes(),
        )
        .unwrap();
    assert!(truncated.finish().is_err());
}

// length와 content_filter는 partial message를 닫되 completed와 다른 terminal 종류를 유지합니다.
#[test]
fn preserves_incomplete_and_failed_terminal_kinds() {
    for (finish_reason, expected) in [
        (
            "length",
            ModelConnectorTerminal::Incomplete {
                reason: Some("length".to_owned()),
                request_failure: yo_core::ModelRequestFailureKind::ResponseLimit,
            },
        ),
        (
            "content_filter",
            ModelConnectorTerminal::Failed {
                code: Some("content_filter".to_owned()),
                request_failure: yo_core::ModelRequestFailureKind::RequestRejected,
            },
        ),
    ] {
        let stream = [
            event(json!({
                "id":"terminal-kind",
                "choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":finish_reason}]
            })),
            final_usage("terminal-kind"),
            "data: [DONE]\n\n".to_owned(),
        ]
        .concat();
        let mut decoder = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
        let mut events = decoder.push(stream.as_bytes()).unwrap();
        events.extend(decoder.finish().unwrap());
        assert!(events.iter().any(|event| matches!(
            event,
            ModelConnectorEvent::MessageDone {
                output_index: 0,
                ..
            }
        )));
        assert!(matches!(
            events.last(),
            Some(ModelConnectorEvent::Terminal { status, .. }) if status == &expected
        ));
    }
}

// 중복 finish와 prompt+completion 합계가 맞지 않는 usage는 protocol failure입니다.
#[test]
fn rejects_duplicate_finish_and_invalid_usage() {
    let mut duplicate_finish = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    duplicate_finish
        .push(
            event(json!({
                "id":"duplicate-finish",
                "choices":[{"index":0,"delta":{"content":"done"},"finish_reason":"stop"}]
            }))
            .as_bytes(),
        )
        .unwrap();
    assert!(
        duplicate_finish
            .push(
                event(json!({
                    "id":"duplicate-finish",
                    "choices":[{"index":0,"delta":{},"finish_reason":"stop"}]
                }))
                .as_bytes()
            )
            .is_err()
    );

    let mut inconsistent_usage = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    inconsistent_usage
        .push(
            event(json!({
                "id":"bad-usage",
                "choices":[{"index":0,"delta":{"content":"done"},"finish_reason":"stop"}]
            }))
            .as_bytes(),
        )
        .unwrap();
    assert!(
        inconsistent_usage
            .push(
                event(json!({
                    "id":"bad-usage",
                    "choices":[],
                    "usage":{"prompt_tokens":4,"completion_tokens":3,"total_tokens":8}
                }))
                .as_bytes()
            )
            .is_err()
    );
}

// usage 전 DONE이나 DONE 뒤 추가 data는 완전한 terminal sequence가 아니므로 거절합니다.
#[test]
fn rejects_done_before_usage_and_data_after_done() {
    let mut early_done = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    early_done
        .push(
            event(json!({
                "id":"early-done",
                "choices":[{"index":0,"delta":{"content":"done"},"finish_reason":"stop"}]
            }))
            .as_bytes(),
        )
        .unwrap();
    assert!(early_done.push(b"data: [DONE]\n\n").is_err());

    let complete = [
        event(json!({
            "id":"tail-data",
            "choices":[{"index":0,"delta":{"content":"done"},"finish_reason":"stop"}]
        })),
        final_usage("tail-data"),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut tail_data = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    tail_data.push(complete.as_bytes()).unwrap();
    assert!(
        tail_data
            .push(
                event(json!({
                    "id":"tail-data",
                    "choices":[],
                    "usage":{"prompt_tokens":4,"completion_tokens":3,"total_tokens":7}
                }))
                .as_bytes()
            )
            .is_err()
    );
}

// UTF-8이 아닌 SSE data와 event byte/count 한도 초과는 dialect 해석 전에 typed failure가 됩니다.
#[test]
fn rejects_invalid_utf8_and_enforces_sse_frame_bounds() {
    let mut invalid = ChatCompletionsSseDecoder::new(ModelConnectorLimits::default());
    assert_eq!(
        invalid.push(b"data: \xff\n\n").unwrap_err().kind(),
        ConnectorFailureKind::Protocol
    );

    let mut oversized = ChatCompletionsSseDecoder::new(ModelConnectorLimits {
        max_sse_event_bytes: 8,
        ..ModelConnectorLimits::default()
    });
    assert_eq!(
        oversized.push(b"data: 123").unwrap_err().kind(),
        ConnectorFailureKind::Limit
    );

    let mut too_many = ChatCompletionsSseDecoder::new(ModelConnectorLimits {
        max_sse_events: 1,
        ..ModelConnectorLimits::default()
    });
    assert_eq!(
        too_many.push(b": one\n\n: two\n\n").unwrap_err().kind(),
        ConnectorFailureKind::Limit
    );
}

// content·refusal·reasoning은 각각의 누적 byte 한도를 추가 bytes 보존 전에 검사합니다.
#[test]
fn enforces_each_cumulative_text_channel_limit() {
    for (field, limits) in [
        (
            "content",
            ModelConnectorLimits {
                max_response_text_bytes: 1,
                ..ModelConnectorLimits::default()
            },
        ),
        (
            "refusal",
            ModelConnectorLimits {
                max_refusal_bytes: 1,
                ..ModelConnectorLimits::default()
            },
        ),
        (
            "reasoning_content",
            ModelConnectorLimits {
                max_reasoning_bytes: 1,
                ..ModelConnectorLimits::default()
            },
        ),
    ] {
        let mut decoder = ChatCompletionsSseDecoder::new(limits);
        let mut delta = serde_json::Map::new();
        delta.insert(field.to_owned(), Value::String("ab".to_owned()));
        let error = decoder
            .push(
                event(json!({
                    "id":"bounded-text",
                    "choices":[{"index":0,"delta":delta,"finish_reason":null}]
                }))
                .as_bytes(),
            )
            .unwrap_err();

        assert_eq!(error.kind(), ConnectorFailureKind::Limit);
    }
}

// tool-call argument bytes와 output-item count는 fragment가 retained call state에 들어가기 전에
// 제한됩니다.
#[test]
fn enforces_function_argument_and_output_item_limits() {
    let mut arguments = ChatCompletionsSseDecoder::new(ModelConnectorLimits {
        max_function_argument_bytes: 1,
        ..ModelConnectorLimits::default()
    });
    let argument_error = arguments
        .push(
            event(json!({
                "id":"bounded-arguments",
                "choices":[{"index":0,"delta":{"tool_calls":[{
                    "index":0,"id":"call-1","type":"function",
                    "function":{"name":"tool","arguments":"ab"}
                }]},"finish_reason":null}]
            }))
            .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(argument_error.kind(), ConnectorFailureKind::Limit);

    let mut items = ChatCompletionsSseDecoder::new(ModelConnectorLimits {
        max_output_items: 1,
        ..ModelConnectorLimits::default()
    });
    let item_error = items
        .push(
            event(json!({
                "id":"bounded-items",
                "choices":[{"index":0,"delta":{"tool_calls":[
                    {"index":0,"id":"call-1","type":"function","function":{"name":"one","arguments":""}},
                    {"index":1,"id":"call-2","type":"function","function":{"name":"two","arguments":""}}
                ]},"finish_reason":null}]
            }))
            .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(item_error.kind(), ConnectorFailureKind::Limit);
}
