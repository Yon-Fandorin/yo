use serde_json::json;
use yo_core::ModelRequestFailureKind;

use super::*;

fn event(value: serde_json::Value) -> String {
    format!("data: {}\n\n", serde_json::to_string(&value).unwrap())
}

fn text_stream() -> String {
    [
        event(json!({
            "type": "response.created",
            "sequence_number": 1,
            "response": {"id": "resp-1"}
        })),
        event(json!({
            "type": "response.output_item.added",
            "sequence_number": 2,
            "output_index": 0,
            "item": {"id": "msg-1", "type": "message", "role": "assistant", "content": []}
        })),
        event(json!({
            "type": "response.output_text.delta",
            "sequence_number": 3,
            "output_index": 0,
            "content_index": 0,
            "item_id": "msg-1",
            "delta": "안녕"
        })),
        event(json!({
            "type": "response.output_text.done",
            "sequence_number": 4,
            "output_index": 0,
            "content_index": 0,
            "item_id": "msg-1",
            "text": "안녕"
        })),
        event(json!({
            "type": "response.output_item.done",
            "sequence_number": 5,
            "output_index": 0,
            "item": {
                "id": "msg-1",
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": "안녕", "annotations": []}]
            }
        })),
        event(json!({
            "type": "response.completed",
            "sequence_number": 6,
            "response": {
                "id": "resp-1",
                "status": "completed",
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 7,
                    "total_tokens": 19,
                    "input_tokens_details": {
                        "cached_tokens": 5,
                        "cache_write_tokens": 99,
                        "x_details": ["non-authoritative"]
                    },
                    "output_tokens_details": {"reasoning_tokens": 3}
                }
            }
        })),
    ]
    .concat()
}

// UTF-8 text delta가 HTTP chunk 경계와 무관하게 exact item correlation으로 복원되고,
// terminal usage, reasoning token, 계약된 cache-read 수만 손실 없이 보고되는지 검증합니다.
#[test]
fn decodes_chunked_text_and_terminal_usage() {
    let stream = text_stream();
    let split = stream.find("안").unwrap() + 1;
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());
    let mut events = decoder.push(&stream.as_bytes()[..split]).unwrap();
    events.extend(decoder.push(&stream.as_bytes()[split..]).unwrap());
    events.extend(decoder.finish().unwrap());

    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::TextDelta { item_id, delta, .. }
            if item_id == "msg-1" && delta == "안녕"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::Terminal {
            response_id,
            status: ModelConnectorTerminal::Completed,
            usage: ModelConnectorUsage {
                input_tokens: Some(12),
                output_tokens: Some(7),
                total_tokens: Some(19),
                reasoning_tokens: Some(3),
                cache_read_input_tokens: yo_core::CacheReadInputTokens::Reported {
                    tokens: 5,
                    source_profile,
                },
            },
        } if response_id == "resp-1"
            && source_profile.as_str()
                == "openai.responses.usage.input-tokens-details.cached-tokens/v1"
    )));
}

// usage 계층의 실제 JSON member가 생략된 경우만 같은 source profile의 absent로
// 보존하고, 보고된 0은 absent와 구별되는 유효한 cache-read 수로 유지합니다.
#[test]
fn cache_read_usage_distinguishes_omission_from_reported_zero() {
    for response in [
        json!({}),
        json!({"usage": {}}),
        json!({"usage": {"input_tokens_details": {}}}),
    ] {
        let usage = usage_at(&json!({"response": response})).unwrap();
        assert!(matches!(
            usage.cache_read_input_tokens,
            yo_core::CacheReadInputTokens::Absent { ref source_profile }
                if source_profile.as_str()
                    == "openai.responses.usage.input-tokens-details.cached-tokens/v1"
        ));
    }

    let usage = usage_at(&json!({
        "response": {
            "usage": {
                "input_tokens": 4,
                "input_tokens_details": {"cached_tokens": 0}
            }
        }
    }))
    .unwrap();
    assert!(matches!(
        usage.cache_read_input_tokens,
        yo_core::CacheReadInputTokens::Reported {
            tokens: 0,
            ref source_profile,
        } if source_profile.as_str()
            == "openai.responses.usage.input-tokens-details.cached-tokens/v1"
    ));
}

// null·잘못된 컨테이너·부호 없는 정수가 아닌 값·input_tokens 부재 및 초과 보고를
// 추측으로 고치지 않고 모두 typed Protocol failure로 닫습니다.
#[test]
fn cache_read_usage_rejects_malformed_or_inconsistent_reports() {
    let invalid_usage = [
        serde_json::Value::Null,
        json!([]),
        json!("invalid"),
        json!({"input_tokens": 4, "input_tokens_details": null}),
        json!({"input_tokens": 4, "input_tokens_details": []}),
        json!({"input_tokens": 4, "input_tokens_details": "invalid"}),
        json!({"input_tokens": 4, "input_tokens_details": {"cached_tokens": null}}),
        json!({"input_tokens": 4, "input_tokens_details": {"cached_tokens": -1}}),
        json!({"input_tokens": 4, "input_tokens_details": {"cached_tokens": 1.5}}),
        json!({"input_tokens": 4, "input_tokens_details": {"cached_tokens": "1"}}),
        json!({"input_tokens_details": {"cached_tokens": 0}}),
        json!({"input_tokens": null, "input_tokens_details": {"cached_tokens": 0}}),
        json!({"input_tokens": -1, "input_tokens_details": {"cached_tokens": 0}}),
        json!({"input_tokens": 1.5, "input_tokens_details": {"cached_tokens": 0}}),
        json!({"input_tokens": "4", "input_tokens_details": {"cached_tokens": 0}}),
        json!({"input_tokens": 4, "input_tokens_details": {"cached_tokens": 5}}),
    ];

    for usage in invalid_usage {
        let error = usage_at(&json!({"response": {"usage": usage}})).unwrap_err();
        assert_eq!(error.kind(), ConnectorFailureKind::Protocol, "{error}");
    }
}

// Responses terminal의 structured reason/code만 closed failure kind로 해석하고, 알 수 없는
// 값은 body text를 추측하지 않은 채 protocol로 축소합니다.
#[test]
fn classifies_terminal_failure_only_from_typed_responses_fields() {
    for (event_type, response, expected) in [
        (
            "response.incomplete",
            json!({
                "id": "resp-terminal",
                "status": "incomplete",
                "incomplete_details": {"reason": "max_output_tokens"}
            }),
            ModelRequestFailureKind::ResponseLimit,
        ),
        (
            "response.failed",
            json!({
                "id": "resp-terminal",
                "status": "failed",
                "error": {"code": "server_error", "message": "private-sentinel"}
            }),
            ModelRequestFailureKind::ProviderUnavailable,
        ),
        (
            "response.failed",
            json!({
                "id": "resp-terminal",
                "status": "failed",
                "error": {"code": "model_not_found", "message": "private-sentinel"}
            }),
            ModelRequestFailureKind::ModelUnavailable,
        ),
        (
            "response.failed",
            json!({
                "id": "resp-terminal",
                "status": "failed",
                "error": {"code": "future_error", "message": "model_not_found"}
            }),
            ModelRequestFailureKind::Protocol,
        ),
    ] {
        let stream = [
            event(json!({
                "type": "response.created",
                "sequence_number": 1,
                "response": {"id": "resp-terminal"}
            })),
            event(json!({
                "type": event_type,
                "sequence_number": 2,
                "response": response
            })),
        ]
        .concat();
        let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());
        let mut events = decoder.push(stream.as_bytes()).unwrap();
        events.extend(decoder.finish().unwrap());

        assert!(matches!(
            events.last(),
            Some(ModelConnectorEvent::Terminal { status, .. })
                if status.request_failure_kind() == Some(expected)
        ));
    }
}

// function call의 item id, call_id, 이름, argument delta는 한 output index에 묶여
// 최종 arguments와 일치할 때만 완료 event가 나오도록 correlation을 검증합니다.
#[test]
fn preserves_function_call_identity_and_exact_argument_bytes() {
    let stream = [
        event(json!({
            "type": "response.created", "sequence_number": 1,
            "response": {"id": "resp-tool"}
        })),
        event(json!({
            "type": "response.output_item.added", "sequence_number": 2,
            "output_index": 0,
            "item": {"id": "call-item", "type": "function_call", "call_id": "call-7", "name": "read_file", "arguments": ""}
        })),
        event(json!({
            "type": "response.function_call_arguments.delta", "sequence_number": 3,
            "output_index": 0, "item_id": "call-item", "delta": "{\"path\":"
        })),
        event(json!({
            "type": "response.function_call_arguments.delta", "sequence_number": 4,
            "output_index": 0, "item_id": "call-item", "delta": "\"src/lib.rs\"}"
        })),
        event(json!({
            "type": "response.function_call_arguments.done", "sequence_number": 5,
            "output_index": 0, "item_id": "call-item", "name": "read_file", "arguments": "{\"path\":\"src/lib.rs\"}"
        })),
        event(json!({
            "type": "response.output_item.done", "sequence_number": 6,
            "output_index": 0,
            "item": {"id": "call-item", "type": "function_call", "call_id": "call-7", "name": "read_file", "arguments": "{\"path\":\"src/lib.rs\"}"}
        })),
        event(json!({
            "type": "response.completed", "sequence_number": 7,
            "response": {"id": "resp-tool", "status": "completed"}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let events = decoder.push(stream.as_bytes()).unwrap();
    decoder.finish().unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::FunctionCallDone {
            item_id,
            call_id,
            name,
            arguments,
            ..
        } if item_id == "call-item"
            && call_id == "call-7"
            && name == "read_file"
            && arguments == "{\"path\":\"src/lib.rs\"}"
    )));
}

// 알려지지 않은 output item은 후속 delta를 임의 해석하거나 무시하지 않고 즉시 typed
// Protocol failure가 되어 model output 의미가 조용히 손실되지 않는지 검증합니다.
#[test]
fn rejects_an_unknown_output_item_type() {
    let stream = event(json!({
        "type": "response.output_item.added",
        "sequence_number": 1,
        "output_index": 0,
        "item": {"id": "built-in-1", "type": "web_search_call"}
    }));
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let error = decoder.push(stream.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// Responses에서도 한 network chunk의 앞선 정상 event는 뒤의 malformed event 때문에
// 사라지지 않고 failure와 함께 반환되어 공용 transport가 먼저 관찰 가능하게 만든다.
#[test]
fn responses_preserves_completed_events_before_a_later_same_chunk_failure() {
    let stream = [
        event(json!({
            "type": "response.created",
            "sequence_number": 1,
            "response": {"id": "resp-partial"}
        })),
        "data: {not-json}\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let batch = decoder.push_batch(stream.as_bytes());

    assert!(matches!(
        batch.failure,
        Some(ref failure) if failure.kind() == ConnectorFailureKind::Protocol
    ));
    assert!(matches!(
        batch.events.as_slice(),
        [ModelConnectorEvent::ResponseCreated { response_id }] if response_id == "resp-partial"
    ));
}

// refusal content도 text와 다른 content index로 정확히 correlation되어 delta가 노출되고,
// 완료 event 없이 message item을 닫을 수 없도록 검증합니다.
#[test]
fn preserves_refusal_content_without_silently_dropping_it() {
    let stream = [
        event(json!({
            "type": "response.output_item.added", "sequence_number": 1,
            "output_index": 0, "item": {"id": "msg", "type": "message"}
        })),
        event(json!({
            "type": "response.refusal.delta", "sequence_number": 2,
            "output_index": 0, "content_index": 1, "item_id": "msg", "delta": "거절"
        })),
        event(json!({
            "type": "response.refusal.done", "sequence_number": 3,
            "output_index": 0, "content_index": 1, "item_id": "msg", "refusal": "거절"
        })),
        event(json!({
            "type": "response.output_item.done", "sequence_number": 4,
            "output_index": 0, "item": {"id": "msg", "type": "message"}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let events = decoder.push(stream.as_bytes()).unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::RefusalDelta { item_id, content_index: 1, delta, .. }
            if item_id == "msg" && delta == "거절"
    )));
}

// provider가 선언한 빈 output_text part도 delta가 없다는 이유로 거절하지 않고,
// part와 item 완료 correlation을 모두 확인한 뒤 정상적으로 끝낼 수 있는지 검증합니다.
#[test]
fn accepts_a_declared_empty_output_text_part() {
    let stream = [
        event(json!({
            "type": "response.output_item.added", "sequence_number": 1,
            "output_index": 0, "item": {"id": "msg", "type": "message"}
        })),
        event(json!({
            "type": "response.content_part.added", "sequence_number": 2,
            "output_index": 0, "content_index": 0, "item_id": "msg",
            "part": {"type": "output_text", "text": "", "annotations": []}
        })),
        event(json!({
            "type": "response.output_text.done", "sequence_number": 3,
            "output_index": 0, "content_index": 0, "item_id": "msg", "text": ""
        })),
        event(json!({
            "type": "response.content_part.done", "sequence_number": 4,
            "output_index": 0, "content_index": 0, "item_id": "msg",
            "part": {"type": "output_text", "text": "", "annotations": []}
        })),
        event(json!({
            "type": "response.output_item.done", "sequence_number": 5,
            "output_index": 0, "item": {"id": "msg", "type": "message"}
        })),
        event(json!({
            "type": "response.completed", "sequence_number": 6,
            "response": {"id": "resp-empty", "status": "completed"}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());
    assert!(matches!(
        events.as_slice(),
        [
            ModelConnectorEvent::MessageDone { item_id, .. },
            ModelConnectorEvent::Terminal { response_id, .. }
        ] if item_id == "msg" && response_id == "resp-empty"
    ));
}

// private reasoning text와 observable summary의 여러 index가 섞여 와도 각각의 wire
// channel·part identity를 보존하고 독립적인 done correlation을 적용하는지 검증합니다.
#[test]
fn preserves_interleaved_reasoning_channels_and_part_indices() {
    let stream = [
        event(json!({
            "type": "response.output_item.added", "sequence_number": 1,
            "output_index": 0, "item": {"id": "reason", "type": "reasoning"}
        })),
        event(json!({
            "type": "response.reasoning_summary_text.delta", "sequence_number": 2,
            "output_index": 0, "summary_index": 1, "item_id": "reason", "delta": "summary"
        })),
        event(json!({
            "type": "response.reasoning_text.delta", "sequence_number": 3,
            "output_index": 0, "content_index": 0, "item_id": "reason", "delta": "private"
        })),
        event(json!({
            "type": "response.reasoning_summary_text.done", "sequence_number": 4,
            "output_index": 0, "summary_index": 1, "item_id": "reason", "text": "summary"
        })),
        event(json!({
            "type": "response.reasoning_text.done", "sequence_number": 5,
            "output_index": 0, "content_index": 0, "item_id": "reason", "text": "private"
        })),
        event(json!({
            "type": "response.output_item.done", "sequence_number": 6,
            "output_index": 0, "item": {"id": "reason", "type": "reasoning"}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let events = decoder.push(stream.as_bytes()).unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::ReasoningDelta {
            channel: ReasoningChannel::Summary,
            part_index: 1,
            delta,
            ..
        } if delta == "summary"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ModelConnectorEvent::ReasoningDelta {
            channel: ReasoningChannel::Text,
            part_index: 0,
            delta,
            ..
        } if delta == "private"
    )));
}

// delta가 없는 빈 reasoning text와 summary도 공식 part wrapper가 indexed state를
// 만들고 각 text done·wrapper done을 정확히 correlation하여 완료되는지 검증합니다.
#[test]
fn accepts_empty_declared_reasoning_text_and_summary_parts() {
    let stream = [
        event(json!({
            "type": "response.output_item.added", "sequence_number": 1,
            "output_index": 0, "item": {"id": "reason", "type": "reasoning"}
        })),
        event(json!({
            "type": "response.content_part.added", "sequence_number": 2,
            "output_index": 0, "content_index": 0, "item_id": "reason",
            "part": {"type": "reasoning_text", "text": ""}
        })),
        event(json!({
            "type": "response.reasoning_text.done", "sequence_number": 3,
            "output_index": 0, "content_index": 0, "item_id": "reason", "text": ""
        })),
        event(json!({
            "type": "response.content_part.done", "sequence_number": 4,
            "output_index": 0, "content_index": 0, "item_id": "reason",
            "part": {"type": "reasoning_text", "text": ""}
        })),
        event(json!({
            "type": "response.reasoning_summary_part.added", "sequence_number": 5,
            "output_index": 0, "summary_index": 1, "item_id": "reason",
            "part": {"type": "summary_text", "text": ""}
        })),
        event(json!({
            "type": "response.reasoning_summary_text.done", "sequence_number": 6,
            "output_index": 0, "summary_index": 1, "item_id": "reason", "text": ""
        })),
        event(json!({
            "type": "response.reasoning_summary_part.done", "sequence_number": 7,
            "output_index": 0, "summary_index": 1, "item_id": "reason",
            "part": {"type": "summary_text", "text": ""}
        })),
        event(json!({
            "type": "response.output_item.done", "sequence_number": 8,
            "output_index": 0, "item": {"id": "reason", "type": "reasoning"}
        })),
        event(json!({
            "type": "response.completed", "sequence_number": 9,
            "response": {"id": "resp-reason", "status": "completed"}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    assert!(decoder.push(stream.as_bytes()).unwrap().is_empty());
    assert!(matches!(
        decoder.finish().unwrap().as_slice(),
        [ModelConnectorEvent::Terminal { response_id, .. }] if response_id == "resp-reason"
    ));
}

// reasoning wrapper의 final text가 누적 delta와 다르면 item 완료 전에 Protocol
// failure로 거절하여 summary/content wrapper mismatch를 조용히 통과시키지 않습니다.
#[test]
fn rejects_a_mismatched_reasoning_part_wrapper() {
    let stream = [
        event(json!({
            "type": "response.output_item.added", "sequence_number": 1,
            "output_index": 0, "item": {"id": "reason", "type": "reasoning"}
        })),
        event(json!({
            "type": "response.reasoning_summary_part.added", "sequence_number": 2,
            "output_index": 0, "summary_index": 0, "item_id": "reason",
            "part": {"type": "summary_text", "text": ""}
        })),
        event(json!({
            "type": "response.reasoning_summary_text.delta", "sequence_number": 3,
            "output_index": 0, "summary_index": 0, "item_id": "reason", "delta": "exact"
        })),
        event(json!({
            "type": "response.reasoning_summary_text.done", "sequence_number": 4,
            "output_index": 0, "summary_index": 0, "item_id": "reason", "text": "exact"
        })),
        event(json!({
            "type": "response.reasoning_summary_part.done", "sequence_number": 5,
            "output_index": 0, "summary_index": 0, "item_id": "reason",
            "part": {"type": "summary_text", "text": "changed"}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let error = decoder.push(stream.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// JSON event 안에 유효하지 않은 UTF-8 byte가 들어오면 lossy 변환 없이 Protocol
// failure로 거절하여 model text와 function arguments의 exact byte 의미를 지킵니다.
#[test]
fn rejects_invalid_utf8_without_lossy_decoding() {
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let error = decoder.push(b"data: {\"type\":\"\xFF\"}\n\n").unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// transport EOF 전에 completed·incomplete·failed 중 어떤 terminal event도 없으면
// 부분 text를 성공으로 바꾸지 않고 Protocol failure로 끝내는지 검증합니다.
#[test]
fn rejects_stream_end_without_a_terminal_response() {
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());
    decoder
        .push(
            event(json!({
                "type": "response.created", "sequence_number": 1,
                "response": {"id": "resp-partial"}
            }))
            .as_bytes(),
        )
        .unwrap();

    let error = decoder.finish().unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// text delta를 받은 message가 output_text.done 없이 item 완료로 넘어가면 terminal까지
// 받아도 부분 text를 완결된 output으로 승인하지 않고 Protocol failure로 거절합니다.
#[test]
fn rejects_an_output_item_that_finishes_before_its_text() {
    let stream = [
        event(json!({
            "type": "response.output_item.added", "sequence_number": 1,
            "output_index": 0, "item": {"id": "msg", "type": "message"}
        })),
        event(json!({
            "type": "response.output_text.delta", "sequence_number": 2,
            "output_index": 0, "content_index": 0, "item_id": "msg", "delta": "partial"
        })),
        event(json!({
            "type": "response.output_item.done", "sequence_number": 3,
            "output_index": 0, "item": {"id": "msg", "type": "message"}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let error = decoder.push(stream.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// terminal response 뒤에 다른 SSE data가 이어지면 첫 완료를 확정하지 않고 Protocol
// failure로 바꿔 duplicate terminal이나 종료 뒤 text를 숨기지 않는지 검증합니다.
#[test]
fn rejects_any_sse_data_after_the_terminal_event() {
    let stream = [
        event(json!({
            "type": "response.completed", "sequence_number": 1,
            "response": {"id": "resp", "status": "completed"}
        })),
        event(json!({
            "type": "response.in_progress", "sequence_number": 2,
            "response": {"id": "resp", "status": "in_progress"}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    let error = decoder.push(stream.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// OpenRouter Responses는 semantic terminal 뒤 transport 종료 sentinel로 `[DONE]`을
// 한 번 더 보낸다. terminal을 대신할 수는 없지만 clean EOF 앞 marker로는 허용한다.
#[test]
fn accepts_one_done_marker_after_the_terminal_event() {
    let stream = [
        event(json!({
            "type": "response.completed", "sequence_number": 1,
            "response": {"id": "resp", "status": "completed"}
        })),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    assert!(decoder.push(stream.as_bytes()).unwrap().is_empty());
    assert!(matches!(
        decoder.finish().unwrap().as_slice(),
        [ModelConnectorEvent::Terminal { response_id, .. }] if response_id == "resp"
    ));
}

// `[DONE]`은 Responses semantic terminal이 아니므로 단독 marker나 중복 marker를
// 성공으로 승격하지 않는다.
#[test]
fn rejects_done_without_a_terminal_or_more_than_once() {
    let mut without_terminal = ResponsesSseDecoder::new(ModelConnectorLimits::default());
    let error = without_terminal.push(b"data: [DONE]\n\n").unwrap_err();
    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);

    let terminal = event(json!({
        "type": "response.completed", "sequence_number": 1,
        "response": {"id": "resp", "status": "completed"}
    }));
    let mut duplicate = ResponsesSseDecoder::new(ModelConnectorLimits::default());
    duplicate.push(terminal.as_bytes()).unwrap();
    duplicate.push(b"data: [DONE]\n\n").unwrap();
    let error = duplicate.push(b"data: [DONE]\n\n").unwrap_err();
    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// terminal과 invalid tail이 서로 다른 HTTP chunk에 들어와도 EOF 전에는 성공 event를
// 노출하지 않고 뒤의 data를 Protocol failure로 판정하여 chunk 경계 불변성을 지킵니다.
#[test]
fn withholds_a_terminal_until_clean_eof_across_push_boundaries() {
    let terminal = event(json!({
        "type": "response.completed", "sequence_number": 1,
        "response": {"id": "resp", "status": "completed"}
    }));
    let tail = event(json!({
        "type": "response.in_progress", "sequence_number": 2,
        "response": {"id": "resp", "status": "in_progress"}
    }));
    let mut decoder = ResponsesSseDecoder::new(ModelConnectorLimits::default());

    assert!(decoder.push(terminal.as_bytes()).unwrap().is_empty());
    let error = decoder.push(tail.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}
