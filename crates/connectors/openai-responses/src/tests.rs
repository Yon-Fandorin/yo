use serde_json::json;
use yo_core::{
    AccountId, ApiCredential, ApiDialect, ConnectorFailureKind, EffectiveModelBinding,
    FunctionTool, ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorLimits,
    ModelConnectorRequest, ModelId, NormalizedEndpoint, ProviderId, ReasoningEffort,
    RequestToolExposure,
};

use super::*;

mod bounds_cancellation;
mod local_tls;
mod transport_lifecycle;

fn responses_binding(model: &str) -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("qwencloud-token-plan").unwrap(),
        ModelId::new(model).unwrap(),
        ApiDialect::OpenAiResponses,
        NormalizedEndpoint::parse(
            "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1",
        )
        .unwrap(),
    )
}

fn response_body(request: &ModelConnectorRequest, model: &str) -> serde_json::Value {
    OpenAiResponsesConnector::new(
        &responses_binding(model),
        ApiCredential::new("test-secret").unwrap(),
        ModelConnectorLimits::default(),
    )
    .unwrap()
    .tokenization_payload(request)
    .unwrap()
}

// QwenCloud Token Plan의 normalized base URL에는 `responses` segment를 정확히 한 번만
// 붙이고 model·credential은 Debug에서 구분하되 API key 원문은 감추는지 검증합니다.
#[test]
fn constructs_the_exact_qwencloud_responses_endpoint_without_exposing_the_key() {
    let connector = OpenAiResponsesConnector::new(
        &responses_binding("qwen3.8max"),
        ApiCredential::new("sk-sensitive-value").unwrap(),
        ModelConnectorLimits::default(),
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
    let request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::enabled(vec![tool]),
        8_192,
        Some(ReasoningEffort::High),
    )
    .unwrap();

    let body = response_body(&request, "qwen3.8max");

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

// assistant refusal replay는 visible content 뒤에 refusal을 정확히 한 번 결합해
// composition 전환 뒤에도 기존 provider-visible Responses 요청 형태를 보존합니다.
#[test]
fn refusal_replay_preserves_the_existing_provider_visible_request() {
    let request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::Assistant,
            content: "visible".to_owned(),
            refusal: Some("declined".to_owned()),
        }],
        RequestToolExposure::disabled(),
        8_192,
        None,
    )
    .unwrap();
    let body = response_body(&request, "qwen3.8max");

    assert_eq!(body["input"][0]["role"], "assistant");
    assert_eq!(body["input"][0]["content"], "visibledeclined");
}

// disabled exposure는 historical function-call replay를 보존하면서 현재 registry의 tools와
// tool_choice만 wire에서 완전히 생략해 현재 도구를 노출하지 않는 요청을 구분합니다.
#[test]
fn disabled_exposure_omits_current_tools_without_dropping_historical_replay() {
    let request = ModelConnectorRequest::new(
        vec![
            ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::Assistant,
                content: String::new(),
                refusal: None,
            },
            ModelConnectorInputItem::FunctionCall {
                call_id: "historical-call".to_owned(),
                name: "old_tool".to_owned(),
                arguments: "{}".to_owned(),
            },
            ModelConnectorInputItem::FunctionCallOutput {
                call_id: "historical-call".to_owned(),
                output: "done".to_owned(),
            },
        ],
        RequestToolExposure::disabled(),
        128,
        None,
    )
    .unwrap();

    let body = response_body(&request, "model");
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    assert_eq!(body["input"][1]["type"], "function_call");
    assert_eq!(body["input"][2]["type"], "function_call_output");
}

// 출력 상한을 알 수 없는 Responses 요청은 숫자를 대신 만들지 않고 wire body에서
// max_output_tokens 전체 필드를 생략해 connector가 provider 기본 동작을 보존합니다.
#[test]
fn responses_omits_the_output_field_when_the_cap_is_unknown() {
    let request = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
            content: "hello".to_owned(),
            refusal: None,
        }],
        RequestToolExposure::disabled(),
        None,
        None,
    )
    .unwrap();

    assert!(
        response_body(&request, "model")
            .get("max_output_tokens")
            .is_none()
    );
}

// enabled는 현재 registry projection을 뜻하므로 빈 목록을 disabled와 같은 의미로
// 축약하지 않고 생성 단계에서 거절합니다.
#[test]
fn enabled_exposure_requires_a_non_empty_registry_projection() {
    let error = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
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
    let error = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
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
    let error = ModelConnectorRequest::new(
        vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::User,
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
