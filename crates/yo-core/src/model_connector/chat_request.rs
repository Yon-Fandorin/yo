use serde_json::{Map, Value, json};

use super::{
    ConnectorError, ConnectorFailureKind, ResponsesInputItem, ResponsesInputRole, ResponsesRequest,
};

pub(super) fn wire_body(request: &ResponsesRequest, model: &str) -> Result<Value, ConnectorError> {
    let mut messages = Vec::new();
    let mut index = 0;
    while index < request.input().len() {
        match &request.input()[index] {
            ResponsesInputItem::Message {
                role: ResponsesInputRole::Assistant,
                content,
                refusal,
            } => {
                let (tool_calls, next) = collect_tool_calls(request.input(), index + 1);
                messages.push(assistant_message(content, refusal.as_deref(), tool_calls));
                index = next;
            },
            ResponsesInputItem::Message {
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
            ResponsesInputItem::FunctionCall { .. } => {
                let (tool_calls, next) = collect_tool_calls(request.input(), index);
                messages.push(assistant_message("", None, tool_calls));
                index = next;
            },
            ResponsesInputItem::FunctionCallOutput { call_id, output } => {
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
                index += 1;
            },
        }
    }

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
        "max_tokens": request.max_output_tokens(),
    });
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

fn collect_tool_calls(input: &[ResponsesInputItem], start: usize) -> (Vec<Value>, usize) {
    let mut index = start;
    let mut tool_calls = Vec::new();
    while let Some(ResponsesInputItem::FunctionCall {
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
    use super::*;
    use crate::{FunctionTool, ReasoningEffort, RequestToolExposure};

    // 한 assistant round의 content·refusal·tool calls가 서로 덮어쓰지 않고 한 message로 직렬화된다.
    #[test]
    fn serializes_mixed_assistant_content_refusal_and_tool_calls_as_one_message() {
        let request = ResponsesRequest::new(
            vec![
                ResponsesInputItem::Message {
                    role: ResponsesInputRole::System,
                    content: "system".to_owned(),
                    refusal: None,
                },
                ResponsesInputItem::Message {
                    role: ResponsesInputRole::Assistant,
                    content: "visible".to_owned(),
                    refusal: Some("declined".to_owned()),
                },
                ResponsesInputItem::FunctionCall {
                    call_id: "call-1".to_owned(),
                    name: "read_file".to_owned(),
                    arguments: r#"{"path":"README.md"}"#.to_owned(),
                },
                ResponsesInputItem::FunctionCallOutput {
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
    }

    // Chat Completions도 disabled exposure에서 현재 tool fields만 생략하고 historical
    // assistant tool_calls와 tool 결과 message는 그대로 replay합니다.
    #[test]
    fn disabled_exposure_omits_current_chat_tools_but_preserves_replay() {
        let request = ResponsesRequest::new(
            vec![
                ResponsesInputItem::FunctionCall {
                    call_id: "call-1".to_owned(),
                    name: "old_tool".to_owned(),
                    arguments: "{}".to_owned(),
                },
                ResponsesInputItem::FunctionCallOutput {
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
}
