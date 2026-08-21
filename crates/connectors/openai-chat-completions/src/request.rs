use serde_json::{Map, Value, json};
use yo_core::{
    ConnectorError, ConnectorFailureKind, ModelConnectorInputItem, ModelConnectorInputRole,
    ModelConnectorRequest,
};

pub(super) fn wire_body(
    request: &ModelConnectorRequest,
    model: &str,
) -> Result<Value, ConnectorError> {
    let mut messages = Vec::new();
    let mut index = 0;
    while index < request.input().len() {
        match &request.input()[index] {
            ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::Assistant,
                content,
                refusal,
            } => {
                let (tool_calls, next) = collect_tool_calls(request.input(), index + 1);
                messages.push(assistant_message(content, refusal.as_deref(), tool_calls));
                index = next;
            },
            ModelConnectorInputItem::Message {
                role,
                content,
                refusal,
            } => {
                if refusal.is_some() {
                    return Err(configuration_failure(
                        "only an assistant replay message may carry refusal bytes",
                    ));
                }
                messages.push(json!({
                    "role": role.as_str(),
                    "content": content,
                }));
                index += 1;
            },
            ModelConnectorInputItem::FunctionCall { .. } => {
                let (tool_calls, next) = collect_tool_calls(request.input(), index);
                messages.push(assistant_message("", None, tool_calls));
                index = next;
            },
            ModelConnectorInputItem::FunctionCallOutput { call_id, output } => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
                index += 1;
            },
            ModelConnectorInputItem::ProviderPrivateAssistant { .. } => {
                return Err(configuration_failure(
                    "provider-private assistant replay requires its provider-specific connector",
                ));
            },
        }
    }

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    if let Some(max_output_tokens) = request.max_output_tokens() {
        body["max_tokens"] = Value::from(max_output_tokens);
    }
    if let Some(tools) = request.tools() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name(),
                            "description": tool.description(),
                            "parameters": tool.parameters(),
                        },
                    })
                })
                .collect(),
        );
        body["tool_choice"] = Value::String("auto".to_owned());
    }
    Ok(body)
}

fn collect_tool_calls(input: &[ModelConnectorInputItem], start: usize) -> (Vec<Value>, usize) {
    let mut index = start;
    let mut tool_calls = Vec::new();
    while let Some(ModelConnectorInputItem::FunctionCall {
        call_id,
        name,
        arguments,
    }) = input.get(index)
    {
        tool_calls.push(json!({
            "id": call_id,
            "type": "function",
            "function": {
                "name": name,
                "arguments": arguments,
            },
        }));
        index += 1;
    }
    (tool_calls, index)
}

fn assistant_message(content: &str, refusal: Option<&str>, tool_calls: Vec<Value>) -> Value {
    let mut message = Map::new();
    message.insert("role".to_owned(), Value::String("assistant".to_owned()));
    message.insert(
        "content".to_owned(),
        if content.is_empty() {
            Value::Null
        } else {
            Value::String(content.to_owned())
        },
    );
    if let Some(refusal) = refusal {
        message.insert("refusal".to_owned(), Value::String(refusal.to_owned()));
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_owned(), Value::Array(tool_calls));
    }
    Value::Object(message)
}

fn configuration_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Configuration, message)
}

#[cfg(test)]
mod tests {
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
}
