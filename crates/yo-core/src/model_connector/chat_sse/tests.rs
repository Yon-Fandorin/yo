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

// refusal delta와 finish·usage·DONE 순서를 모두 만족한 stream만 resumable terminal을 낸다.
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
    let mut decoder = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());
    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());
    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::RefusalDelta { delta, .. } if delta == "declined"
    )));
    assert!(matches!(
        events.last(),
        Some(ResponsesEvent::Terminal {
            status: ResponseTerminal::Completed,
            ..
        })
    ));
}

// role-only delta 뒤 곧바로 stop이 와도 도구 호출이 없다면 빈 assistant 응답으로
// 완료하며, usage와 DONE까지 갖춘 정확한 terminal sequence를 허용한다.
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
    let mut decoder = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());

    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());

    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::MessageDone {
            output_index: 0,
            ..
        }
    )));
    assert!(matches!(
        events.last(),
        Some(ResponsesEvent::Terminal {
            status: ResponseTerminal::Completed,
            ..
        })
    ));
}

// 하나의 network chunk에서 완전한 delta 뒤 event가 깨져도 먼저 해석된 관찰은
// failure와 함께 반환하여 transport가 Live Projection에 보낸 뒤 Turn을 실패시킨다.
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
    let mut decoder = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());

    let batch = decoder.push_batch(stream.as_bytes());

    assert!(matches!(
        batch.failure,
        Some(ref failure) if failure.kind() == ConnectorFailureKind::Protocol
    ));
    assert!(batch.events.iter().any(|event| matches!(
        event,
        ResponsesEvent::TextDelta { delta, .. } if delta == "visible"
    )));
}

// 같은 assistant round의 visible content와 fragmented tool call을 각각 손실 없이 복원한다.
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
    let mut decoder = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());
    let events = decoder.push(stream.as_bytes()).unwrap();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ResponsesEvent::MessageDone { .. }))
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::FunctionCallDone { arguments, .. }
            if arguments == r#"{"path":"README.md"}"#
    )));
    assert!(decoder.finish().is_ok());
}

// response identity 변경, final usage 누락, stream 절단은 정상 종료로 승격되지 않는다.
#[test]
fn rejects_missing_usage_changed_ids_and_truncated_streams() {
    let mut changed = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());
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

    let mut truncated = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());
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

// length와 content_filter는 partial message를 닫아 backend 상관관계를 완성하되,
// completed와 구분되는 terminal 종류를 유지해 partial replay를 커밋하지 않게 한다.
#[test]
fn preserves_incomplete_and_failed_terminal_kinds() {
    for (finish_reason, expected) in [
        (
            "length",
            ResponseTerminal::Incomplete {
                reason: Some("length".to_owned()),
            },
        ),
        (
            "content_filter",
            ResponseTerminal::Failed {
                code: Some("content_filter".to_owned()),
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
        let mut decoder = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());
        let mut events = decoder.push(stream.as_bytes()).unwrap();
        events.extend(decoder.finish().unwrap());
        assert!(events.iter().any(|event| matches!(
            event,
            ResponsesEvent::MessageDone {
                output_index: 0,
                ..
            }
        )));
        assert!(matches!(
            events.last(),
            Some(ResponsesEvent::Terminal { status, .. }) if status == &expected
        ));
    }
}

// finish reason 중복과 prompt+completion 합계가 맞지 않는 usage는 protocol failure다.
#[test]
fn rejects_duplicate_finish_and_invalid_usage() {
    let mut duplicate_finish = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());
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

    let mut inconsistent_usage =
        ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());
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

// usage 전 DONE이나 DONE 뒤 추가 data는 완전한 terminal sequence가 아니므로 거부한다.
#[test]
fn rejects_done_before_usage_and_data_after_done() {
    let mut early_done = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());
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
    let mut tail_data = ChatCompletionsSseDecoder::new(ResponsesConnectorLimits::default());
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
