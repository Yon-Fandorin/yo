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
mod tests;
