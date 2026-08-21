use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::{
    ConnectorError, KimiWireProfile, ModelConnectorInputItem, ModelConnectorInputRole,
    ModelConnectorRequest, configuration_failure,
};
use crate::private_replay::{decode_envelope, private_message_value};

pub(super) fn messages(
    request: &ModelConnectorRequest,
    profile: KimiWireProfile,
) -> Result<Vec<Value>, ConnectorError> {
    let mut messages = Vec::new();
    let mut known_calls = BTreeSet::new();
    let mut answered_calls = BTreeSet::new();
    let mut index = 0;
    while index < request.input().len() {
        match &request.input()[index] {
            ModelConnectorInputItem::Message {
                role: ModelConnectorInputRole::Assistant,
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
                    Some(ModelConnectorInputItem::FunctionCall { .. })
                ) {
                    index += 1;
                }
                for item in &request.input()[group_start + 1..index] {
                    let ModelConnectorInputItem::FunctionCall { call_id, .. } = item else {
                        unreachable!("assistant call group contains only function calls")
                    };
                    if !known_calls.insert(call_id.as_str()) {
                        return Err(configuration_failure(
                            "Kimi replay contains a duplicate function call identity",
                        ));
                    }
                }
                if profile.private_replay() {
                    let Some(ModelConnectorInputItem::ProviderPrivateAssistant { envelope }) =
                        request.input().get(index)
                    else {
                        return Err(configuration_failure(
                            "Kimi private replay is missing its assistant message",
                        ));
                    };
                    let message = decode_envelope(envelope)?;
                    if !message.is_valid()
                        || message.content().unwrap_or_default() != content
                        || message.tool_calls().len() != index - group_start - 1
                        || !message
                            .tool_calls()
                            .iter()
                            .zip(&request.input()[group_start + 1..index])
                            .all(|(private, generic)| {
                                matches!(
                                    generic,
                                    ModelConnectorInputItem::FunctionCall {
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
                    messages.push(private_message_value(&message));
                    index += 1;
                } else {
                    messages.push(generic_assistant(request.input(), group_start, index)?);
                }
            },
            ModelConnectorInputItem::Message {
                role,
                content,
                refusal,
            } => {
                if refusal.is_some() || *role == ModelConnectorInputRole::Developer {
                    return Err(configuration_failure(
                        "Kimi replay admits only system and user non-assistant messages",
                    ));
                }
                messages.push(json!({"role": role.as_str(), "content": content}));
                index += 1;
            },
            ModelConnectorInputItem::FunctionCall { .. }
            | ModelConnectorInputItem::ProviderPrivateAssistant { .. } => {
                return Err(configuration_failure(
                    "Kimi replay contains an unpaired assistant item",
                ));
            },
            ModelConnectorInputItem::FunctionCallOutput { call_id, output } => {
                if !known_calls.contains(call_id.as_str()) {
                    return Err(configuration_failure(
                        "Kimi replay output has no prior matching function call",
                    ));
                }
                if !answered_calls.insert(call_id.as_str()) {
                    return Err(configuration_failure(
                        "Kimi replay contains a duplicate function call output",
                    ));
                }
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
                index += 1;
            },
        }
    }
    if known_calls.difference(&answered_calls).next().is_some() {
        return Err(configuration_failure(
            "Kimi replay ends with an unanswered function call",
        ));
    }
    Ok(messages)
}

fn generic_assistant(
    input: &[ModelConnectorInputItem],
    start: usize,
    end: usize,
) -> Result<Value, ConnectorError> {
    let ModelConnectorInputItem::Message { content, .. } = &input[start] else {
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

fn generic_tool_call(item: &ModelConnectorInputItem) -> Result<Value, ConnectorError> {
    let ModelConnectorInputItem::FunctionCall {
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
