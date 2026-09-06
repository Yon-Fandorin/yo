use yo_core::{FunctionTool, ReasoningEffort, RequestToolExposure};

use super::*;

// assistant content·refusal·tool calls를 한 message로 직렬화하고 Chat 전용 field만 보냅니다.
#[test]
fn serializes_mixed_assistant_content_refusal_and_tool_calls_as_one_message() {
    let request = ModelConnectorRequest::new(
        vec![
            ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::System,
                content: "system".to_owned(),
                refusal: None,
            },
            ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::Assistant,
                content: "visible".to_owned(),
                refusal: Some("declined".to_owned()),
            },
            ModelConnectorInputItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"README.md"}"#.to_owned(),
            },
            ModelConnectorInputItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "contents".to_owned(),
            },
        ],
        RequestToolExposure::enabled(vec![
            FunctionTool::new("read_file", "read one file", json!({"type":"object"})).unwrap(),
        ]),
        512,
        Some(ReasoningEffort::High),
    )
    .unwrap();

    let body = wire_body(&request, "deepseek-v4-flash-0731").unwrap();
    assert_eq!(body["messages"][1]["content"], "visible");
    assert_eq!(body["messages"][1]["refusal"], "declined");
    assert_eq!(body["messages"][1]["tool_calls"][0]["id"], "call-1");
    assert_eq!(body["messages"][2]["role"], "tool");
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["max_tokens"], 512);
    assert!(body.get("reasoning").is_none());
    assert!(body.get("enable_thinking").is_none());
    assert!(body.get("prompt_cache_key").is_none());
}

// disabled exposure는 현재 tools만 생략하고 historical call/result replay는 보존합니다.
#[test]
fn disabled_exposure_omits_current_chat_tools_but_preserves_replay() {
    let request = ModelConnectorRequest::new(
        vec![
            ModelConnectorInputItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "old_tool".to_owned(),
                arguments: "{}".to_owned(),
            },
            ModelConnectorInputItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "done".to_owned(),
            },
        ],
        RequestToolExposure::disabled(),
        128,
        None,
    )
    .unwrap();

    let body = wire_body(&request, "model").unwrap();
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call-1");
    assert_eq!(body["messages"][1]["role"], "tool");
}

// unknown output cap은 임의 값으로 치환하지 않고 max_tokens를 완전히 생략합니다.
#[test]
fn unknown_output_cap_omits_chat_max_tokens() {
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
        wire_body(&request, "model")
            .unwrap()
            .get("max_tokens")
            .is_none()
    );
}
