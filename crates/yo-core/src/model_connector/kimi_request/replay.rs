use serde_json::{Value, json};

use super::{
    ConnectorError, KIMI_ASSISTANT_SCHEMA, KimiWireProfile, ResponsesInputItem, ResponsesInputRole,
    ResponsesRequest, configuration_failure,
};

pub(super) fn messages(
    request: &ResponsesRequest,
    profile: KimiWireProfile,
) -> Result<Vec<Value>, ConnectorError> {
    let mut messages = Vec::new();
    let mut index = 0;
    while index < request.input().len() {
        match &request.input()[index] {
            ResponsesInputItem::Message {
                role: ResponsesInputRole::Assistant,
                content,
                refusal,
            } => {
                if refusal.is_some() {
                    return Err(configuration_failure(
                        "Kimi assistant replay does not admit refusal",
                    ));
                }
                let group_start = index;
                index += 1;
                while matches!(
                    request.input().get(index),
                    Some(ResponsesInputItem::FunctionCall { .. })
                ) {
                    index += 1;
                }
                if profile.private_replay() {
                    let Some(ResponsesInputItem::ProviderPrivateAssistant { schema, message }) =
                        request.input().get(index)
                    else {
                        return Err(configuration_failure(
                            "Kimi private replay is missing its assistant message",
                        ));
                    };
                    if schema != KIMI_ASSISTANT_SCHEMA
                        || !message.is_valid()
                        || message.content().unwrap_or_default() != content
                        || message.tool_calls().len() != index - group_start - 1
                        || !message
                            .tool_calls()
                            .iter()
                            .zip(&request.input()[group_start + 1..index])
                            .all(|(private, generic)| {
                                matches!(
                                    generic,
                                    ResponsesInputItem::FunctionCall {
                                        call_id,
                                        name,
                                        arguments,
                                    } if private.id() == call_id
                                        && private.name() == name
                                        && private.arguments() == arguments
                                )
                            })
                    {
                        return Err(configuration_failure(
                            "Kimi private assistant differs from its semantic projection",
                        ));
                    }
                    messages.push(private_message(message));
                    index += 1;
                } else {
                    messages.push(generic_assistant(request.input(), group_start, index)?);
                }
            },
            ResponsesInputItem::Message {
                role,
                content,
                refusal,
            } => {
                if refusal.is_some() || *role == ResponsesInputRole::Developer {
                    return Err(configuration_failure(
                        "Kimi replay admits only system and user non-assistant messages",
                    ));
                }
                messages.push(json!({"role": role.as_str(), "content": content}));
                index += 1;
            },
            ResponsesInputItem::FunctionCall { .. }
            | ResponsesInputItem::ProviderPrivateAssistant { .. } => {
                return Err(configuration_failure(
                    "Kimi replay contains an unpaired assistant item",
                ));
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
    Ok(messages)
}

fn generic_assistant(
    input: &[ResponsesInputItem],
    start: usize,
    end: usize,
) -> Result<Value, ConnectorError> {
    let ResponsesInputItem::Message { content, .. } = &input[start] else {
        unreachable!("generic assistant starts at a message")
    };
    let mut value = json!({"role": "assistant", "content": content});
    if end > start + 1 {
        value["tool_calls"] = Value::Array(
            input[start + 1..end]
                .iter()
                .map(generic_tool_call)
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok(value)
}

fn generic_tool_call(item: &ResponsesInputItem) -> Result<Value, ConnectorError> {
    let ResponsesInputItem::FunctionCall {
        call_id,
        name,
        arguments,
    } = item
    else {
        return Err(configuration_failure(
            "Kimi assistant tool projection is malformed",
        ));
    };
    Ok(json!({
        "id": call_id,
        "type": "function",
        "function": {"name": name, "arguments": arguments},
    }))
}

fn private_message(message: &crate::KimiAssistantMessage) -> Value {
    let mut value = json!({
        "role": "assistant",
        "reasoning_content": message.reasoning_content(),
        "content": message.content(),
    });
    if !message.tool_calls().is_empty() {
        value["tool_calls"] = Value::Array(
            message
                .tool_calls()
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id(),
                        "type": "function",
                        "function": {
                            "name": call.name(),
                            "arguments": call.arguments(),
                        },
                    })
                })
                .collect(),
        );
    }
    value
}
