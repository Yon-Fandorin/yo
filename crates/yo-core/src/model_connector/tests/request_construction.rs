use serde_json::json;

use super::{
    super::{
        ConnectorFailureKind, ResponsesConnectorLimits,
        connector::{OpenAiChatCompletionsConnector, OpenAiResponsesConnector},
        request::{
            FunctionTool, ReasoningEffort, RequestToolExposure, ResponsesInputItem,
            ResponsesInputRole, ResponsesRequest,
        },
    },
    support::qwen_binding,
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
            refusal: None,
        }],
        RequestToolExposure::enabled(vec![tool]),
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

// disabled exposure는 historical function-call replay를 보존하면서 현재 registry의 tools와
// tool_choice만 wire에서 완전히 생략해 no-tools/verification 요청을 구분합니다.
#[test]
fn disabled_exposure_omits_current_tools_without_dropping_historical_replay() {
    let request = ResponsesRequest::new(
        vec![
            ResponsesInputItem::Message {
                role: ResponsesInputRole::Assistant,
                content: String::new(),
                refusal: None,
            },
            ResponsesInputItem::FunctionCall {
                call_id: "historical-call".to_owned(),
                name: "old_tool".to_owned(),
                arguments: "{}".to_owned(),
            },
            ResponsesInputItem::FunctionCallOutput {
                call_id: "historical-call".to_owned(),
                output: "done".to_owned(),
            },
        ],
        RequestToolExposure::disabled(),
        128,
        None,
    )
    .unwrap();

    let body = request.wire_body("model");
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    assert_eq!(body["input"][1]["type"], "function_call");
    assert_eq!(body["input"][2]["type"], "function_call_output");
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
