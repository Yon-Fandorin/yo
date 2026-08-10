use serde_json::json;

use super::{
    super::{
        ConnectorFailureKind, ResponsesCancellation, ResponsesConnectorLimits,
        connector::OpenAiResponsesConnector,
        request::{ResponsesInputItem, ResponsesInputRole, ResponsesRequest},
        sse::ResponsesSseDecoder,
    },
    support::{event, qwen_binding},
};
use crate::ApiCredential;

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
            refusal: None,
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
