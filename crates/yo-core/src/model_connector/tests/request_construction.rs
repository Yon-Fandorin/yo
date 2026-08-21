use super::super::{
    ConnectorFailureKind,
    request::{RequestToolExposure, ResponsesInputItem, ResponsesInputRole, ResponsesRequest},
};

// enabled는 현재 registry projection을 뜻하므로 빈 목록을 disabled와 같은 의미로
// 축약하지 않고 생성 단계에서 거절합니다.
#[test]
fn enabled_exposure_requires_a_non_empty_registry_projection() {
    let error = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::enabled(Vec::new()),
        128,
        None,
    )
    .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
}

// output cap 0은 무제한처럼 해석하지 않고 network dispatch 전에 configuration 오류로
// 거절합니다.
#[test]
fn rejects_a_zero_responses_output_cap() {
    let error = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        0,
        None,
    )
    .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
}

// connector 입력을 직접 만드는 호출자도 user·system 메시지에 refusal을 붙일 수 없으며,
// dialect별 serializer가 같은 replay를 서로 다르게 해석하기 전에 공통 생성자가 거부한다.
#[test]
fn rejects_visible_refusal_on_a_non_assistant_connector_message() {
    let error = ResponsesRequest::new(
        vec![ResponsesInputItem::Message {
            role: ResponsesInputRole::User,
            content: "hello".to_owned(),
            refusal: Some("declined".to_owned()),
        }],
        RequestToolExposure::disabled(),
        8_192,
        None,
    )
    .unwrap_err();

    assert_eq!(error.kind(), ConnectorFailureKind::Configuration);
}
