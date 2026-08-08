use serde_json::json;

use super::{
    connector::OpenAiResponsesConnector,
    request::{
        FunctionTool, ReasoningEffort, ResponsesInputItem, ResponsesInputRole, ResponsesRequest,
    },
    sse::ResponsesSseDecoder,
    *,
};
use crate::{
    AccountId, ApiCredential, ApiDialect, ConnectorId, EffectiveModelBinding, ModelId,
    NormalizedEndpoint, ProviderId,
};

fn qwen_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("qwencloud-token-plan").unwrap(),
        ModelId::new("qwen3.8max").unwrap(),
        ConnectorId::new("openai-responses").unwrap(),
        ApiDialect::OpenAiResponses,
        NormalizedEndpoint::parse(
            "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1",
        )
        .unwrap(),
    )
}

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
                    "output_tokens_details": {"reasoning_tokens": 3}
                }
            }
        })),
    ]
    .concat()
}

// QwenCloud Token Plan의 normalized base URL에는 `responses` segment를 정확히 한 번만
// 붙이고 model·credential은 Debug에서 구분하되 API key 원문은 감추는지 검증합니다.
#[test]
fn constructs_the_exact_qwencloud_responses_endpoint_without_exposing_the_key() {
    let connector = OpenAiResponsesConnector::new(
        &qwen_binding(),
        ApiCredential::new("sk-sensitive-value").unwrap(),
        ResponsesConnectorLimits::default(),
    )
    .unwrap();

    assert_eq!(
        connector.request_url(),
        "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/responses"
    );
    let debug = format!("{connector:?}");
    assert!(debug.contains("qwen3.8max"));
    assert!(!debug.contains("sk-sensitive-value"));
}

// request wire body는 선택 binding의 model, 명시적 input·function tool·reasoning effort와
// stream만 포함하고 provider cache나 previous response authority를 암묵적으로 켜지 않습니다.
#[test]
fn serializes_only_the_declared_responses_request_capabilities() {
    let tool = FunctionTool::new(
        "read_file",
        "Read one workspace file",
        json!({"type": "object", "properties": {"path": {"type": "string"}}}),
    )
    .unwrap();
    let request = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
        }],
        vec![tool],
        8_192,
        Some(ReasoningEffort::High),
    )
    .unwrap();

    let body = request.wire_body("qwen3.8max");

    assert_eq!(body["model"], "qwen3.8max");
    assert_eq!(body["stream"], true);
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["max_output_tokens"], 8_192);
    assert_eq!(body["tools"][0]["name"], "read_file");
    assert_eq!(body["reasoning"]["effort"], "high");
    assert!(body.get("previous_response_id").is_none());
    assert!(body.get("conversation").is_none());
    assert!(body.get("x-dashscope-session-cache").is_none());
}

// output cap 0은 무제한처럼 해석하지 않고 network dispatch 전에 configuration 오류로
// 거절합니다.
#[test]
fn rejects_a_zero_responses_output_cap() {
    let error = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
        }],
        Vec::new(),
        0,
        None,
    )
    .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
}

// UTF-8 text delta가 HTTP chunk 경계와 무관하게 exact item correlation으로 복원되고,
// terminal usage와 reasoning token 수까지 손실 없이 보고되는지 검증합니다.
#[test]
fn decodes_chunked_text_and_terminal_usage() {
    let stream = text_stream();
    let split = stream.find("안").unwrap() + 1;
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());
    let mut events = decoder.push(&stream.as_bytes()[..split]).unwrap();
    events.extend(decoder.push(&stream.as_bytes()[split..]).unwrap());
    events.extend(decoder.finish().unwrap());

    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::TextDelta { item_id, delta, .. }
            if item_id == "msg-1" && delta == "안녕"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::Terminal {
            response_id,
            status: ResponseTerminal::Completed,
            usage: ResponsesUsage {
                input_tokens: Some(12),
                output_tokens: Some(7),
                total_tokens: Some(19),
                reasoning_tokens: Some(3),
            },
        } if response_id == "resp-1"
    )));
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
            "response": {"id": "resp-tool", "status": "completed", "usage": null}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    let events = decoder.push(stream.as_bytes()).unwrap();
    decoder.finish().unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::FunctionCallDone {
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
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    let error = decoder.push(stream.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// SSE event delimiter가 오기 전 수신 byte가 profile 한도를 넘으면 전체 event를 더
// buffer하지 않고 Limit failure로 중단하여 원격 입력이 메모리를 무제한 쓰지 않습니다.
#[test]
fn enforces_the_sse_event_byte_limit_while_receiving() {
    let limits = ResponsesConnectorLimits {
        max_sse_event_bytes: 8,
        ..ResponsesConnectorLimits::default()
    };
    let mut decoder = ResponsesSseDecoder::new(limits);

    let error = decoder.push(b"data: 123").unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Limit);
}

// 여러 event에 나뉜 text delta의 누적 UTF-8 byte 수가 profile 한도를 넘으면 각 event가
// 작더라도 Limit failure가 되어 cumulative response bound를 우회하지 못합니다.
#[test]
fn enforces_the_cumulative_response_text_limit() {
    let limits = ResponsesConnectorLimits {
        max_response_text_bytes: 5,
        ..ResponsesConnectorLimits::default()
    };
    let mut decoder = ResponsesSseDecoder::new(limits);
    let prefix = [
        event(json!({
            "type": "response.output_item.added", "sequence_number": 1,
            "output_index": 0,
            "item": {"id": "msg", "type": "message"}
        })),
        event(json!({
            "type": "response.output_text.delta", "sequence_number": 2,
            "output_index": 0, "content_index": 0, "item_id": "msg", "delta": "abc"
        })),
    ]
    .concat();
    decoder.push(prefix.as_bytes()).unwrap();
    let overflow = event(json!({
        "type": "response.output_text.delta", "sequence_number": 3,
        "output_index": 0, "content_index": 0, "item_id": "msg", "delta": "def"
    }));

    let error = decoder.push(overflow.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Limit);
}

// 여러 event에 나뉜 function argument delta가 누적 한도를 넘으면 작은 event들로
// 제한을 우회하지 못하고 typed Limit failure로 중단되는지 검증합니다.
#[test]
fn enforces_the_cumulative_function_argument_limit() {
    let limits = ResponsesConnectorLimits {
        max_function_argument_bytes: 5,
        ..ResponsesConnectorLimits::default()
    };
    let mut decoder = ResponsesSseDecoder::new(limits);
    let prefix = [
        event(json!({
            "type": "response.output_item.added", "sequence_number": 1,
            "output_index": 0,
            "item": {"id": "call", "type": "function_call", "call_id": "call-1", "name": "tool", "arguments": ""}
        })),
        event(json!({
            "type": "response.function_call_arguments.delta", "sequence_number": 2,
            "output_index": 0, "item_id": "call", "delta": "abc"
        })),
    ]
    .concat();
    decoder.push(prefix.as_bytes()).unwrap();
    let overflow = event(json!({
        "type": "response.function_call_arguments.delta", "sequence_number": 3,
        "output_index": 0, "item_id": "call", "delta": "def"
    }));

    let error = decoder.push(overflow.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Limit);
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
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    let events = decoder.push(stream.as_bytes()).unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::RefusalDelta { item_id, content_index: 1, delta, .. }
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
            "response": {"id": "resp-empty", "status": "completed", "usage": null}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    let mut events = decoder.push(stream.as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());
    assert!(matches!(
        events.as_slice(),
        [
            ResponsesEvent::MessageDone { item_id, .. },
            ResponsesEvent::Terminal { response_id, .. }
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
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    let events = decoder.push(stream.as_bytes()).unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::ReasoningDelta {
            channel: ReasoningChannel::Summary,
            part_index: 1,
            delta,
            ..
        } if delta == "summary"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        ResponsesEvent::ReasoningDelta {
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
            "response": {"id": "resp-reason", "status": "completed", "usage": null}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    assert!(decoder.push(stream.as_bytes()).unwrap().is_empty());
    assert!(matches!(
        decoder.finish().unwrap().as_slice(),
        [ResponsesEvent::Terminal { response_id, .. }] if response_id == "resp-reason"
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
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    let error = decoder.push(stream.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// JSON event 안에 유효하지 않은 UTF-8 byte가 들어오면 lossy 변환 없이 Protocol
// failure로 거절하여 model text와 function arguments의 exact byte 의미를 지킵니다.
#[test]
fn rejects_invalid_utf8_without_lossy_decoding() {
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    let error = decoder.push(b"data: {\"type\":\"\xFF\"}\n\n").unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// transport EOF 전에 completed·incomplete·failed 중 어떤 terminal event도 없으면
// 부분 text를 성공으로 바꾸지 않고 Protocol failure로 끝내는지 검증합니다.
#[test]
fn rejects_stream_end_without_a_terminal_response() {
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());
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
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

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
            "response": {"id": "resp", "status": "completed", "usage": null}
        })),
        event(json!({
            "type": "response.in_progress", "sequence_number": 2,
            "response": {"id": "resp", "status": "in_progress"}
        })),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

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
            "response": {"id": "resp", "status": "completed", "usage": null}
        })),
        "data: [DONE]\n\n".to_owned(),
    ]
    .concat();
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    assert!(decoder.push(stream.as_bytes()).unwrap().is_empty());
    assert!(matches!(
        decoder.finish().unwrap().as_slice(),
        [ResponsesEvent::Terminal { response_id, .. }] if response_id == "resp"
    ));
}

// `[DONE]`은 Responses semantic terminal이 아니므로 단독 marker나 중복 marker를
// 성공으로 승격하지 않는다.
#[test]
fn rejects_done_without_a_terminal_or_more_than_once() {
    let mut without_terminal = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());
    let error = without_terminal.push(b"data: [DONE]\n\n").unwrap_err();
    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);

    let terminal = event(json!({
        "type": "response.completed", "sequence_number": 1,
        "response": {"id": "resp", "status": "completed", "usage": null}
    }));
    let mut duplicate = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());
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
        "response": {"id": "resp", "status": "completed", "usage": null}
    }));
    let tail = event(json!({
        "type": "response.in_progress", "sequence_number": 2,
        "response": {"id": "resp", "status": "in_progress"}
    }));
    let mut decoder = ResponsesSseDecoder::new(ResponsesConnectorLimits::default());

    assert!(decoder.push(terminal.as_bytes()).unwrap().is_empty());
    let error = decoder.push(tail.as_bytes()).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Protocol);
}

// 이미 취소된 token은 DNS나 HTTP 연결을 시작하기 전에 typed Cancelled failure를
// 반환하여 취소 뒤 새 원격 작업을 만들지 않는지 검증합니다.
#[test]
fn rejects_a_cancelled_request_before_starting_network_work() {
    let connector = OpenAiResponsesConnector::new(
        &qwen_binding(),
        ApiCredential::new("sk-sensitive-value").unwrap(),
        ResponsesConnectorLimits::default(),
    )
    .unwrap();
    let request = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
        }],
        Vec::new(),
        8_192,
        None,
    )
    .unwrap();
    let cancellation = ResponsesCancellation::new();
    cancellation.cancel();

    let error = connector.start(request, cancellation).unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Cancelled);
}

// 0인 deadline이나 byte/count bound가 하나라도 있으면 HTTP client를 만들기 전에
// Configuration failure로 거절하여 사실상 무제한 profile을 허용하지 않습니다.
#[test]
fn rejects_a_connector_profile_with_a_zero_bound() {
    let limits = ResponsesConnectorLimits {
        max_sse_events: 0,
        ..ResponsesConnectorLimits::default()
    };

    let error = OpenAiResponsesConnector::new(
        &qwen_binding(),
        ApiCredential::new("sk-sensitive-value").unwrap(),
        limits,
    )
    .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
}
