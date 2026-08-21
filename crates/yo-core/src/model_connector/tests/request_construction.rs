use super::super::{
    ConnectorFailureKind, ResponsesConnectorLimits,
    connector::OpenAiChatCompletionsConnector,
    request::{RequestToolExposure, ResponsesInputItem, ResponsesInputRole, ResponsesRequest},
};
use crate::{
    AccountId, ApiCredential, ApiDialect, EffectiveModelBinding, ModelId, NormalizedEndpoint,
    ProviderId,
};

fn deepseek_chat_binding() -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("token-plan").unwrap(),
        ModelId::new("deepseek-v4-flash-0731").unwrap(),
        ApiDialect::OpenAiChatCompletions,
        NormalizedEndpoint::parse("https://dashscope-intl.aliyuncs.com/compatible-mode/v1")
            .unwrap(),
    )
}

// Chat connector는 base URL에 정확한 두 segment만 붙이고 Debug에서 credential을 숨긴다.
#[test]
fn chat_connector_appends_exactly_chat_completions_and_redacts_credentials() {
    let connector = OpenAiChatCompletionsConnector::new(
        &deepseek_chat_binding(),
        ApiCredential::new("secret-token").unwrap(),
        ResponsesConnectorLimits::default(),
    )
    .unwrap();
    assert_eq!(
        connector.request_url(),
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions"
    );
    let debug = format!("{connector:?}");
    assert!(!debug.contains("secret-token"));
}

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
