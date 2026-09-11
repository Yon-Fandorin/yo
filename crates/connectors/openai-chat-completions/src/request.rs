use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use yo_core::{
    ConnectorError, ConnectorFailureKind, ModelConnectorInputItem, ModelConnectorInputRole,
    ModelConnectorRequest,
};

/// Service-admitted request options; never inferred from a provider name at dispatch.
#[derive(Clone)]
pub(super) struct ImageWirePolicy {
    parameters: Value,
    maximum_output: u64,
    local_tools: bool,
}

impl ImageWirePolicy {
    pub(super) fn admit(
        complete: &yo_core::CompleteModelBinding,
    ) -> Result<Option<Self>, ConnectorError> {
        let admitted =
            yo_core::admit_standard_complete_binding(complete).map_err(configuration_failure)?;
        let profile = complete.profile();
        if profile.image_input_profile().is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            parameters: profile.optional_request_parameters().to_json_value(),
            maximum_output: profile.context().max_output_tokens().ok_or_else(|| {
                configuration_failure("image profile requires a known output maximum")
            })?,
            local_tools: admitted.profile().tool_policy()
                == yo_core::AdmittedToolPolicy::LocalTools,
        }))
    }
}

#[cfg(test)]
pub(super) fn wire_body(
    request: &ModelConnectorRequest,
    model: &str,
) -> Result<Value, ConnectorError> {
    projected_body(request, model, None, false)
}

pub(super) fn projected_body(
    request: &ModelConnectorRequest,
    model: &str,
    images: Option<&ImageWirePolicy>,
    tokenization: bool,
) -> Result<Value, ConnectorError> {
    if let Some(policy) = images
        && (!request
            .max_output_tokens()
            .is_some_and(|cap| cap > 0 && cap <= policy.maximum_output)
            || (request.tools().is_some() && !policy.local_tools)
            || request.reasoning_effort().is_some())
    {
        return Err(configuration_failure(
            "request differs from its admitted image profile",
        ));
    }
    let mut messages = Vec::new();
    let mut index = 0;
    while index < request.input().len() {
        match &request.input()[index] {
            ModelConnectorInputItem::MultimodalUser { parts } => {
                messages.push(image_message(parts, images, tokenization)?);
                index += 1;
            },
            ModelConnectorInputItem::ImageSummarySource { source } => {
                messages.push(image_message(source.parts(), images, tokenization)?);
                index += 1;
            },
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
    if let Some(policy) = images {
        for (key, value) in policy.parameters.as_object().expect("admitted mapping") {
            if body.as_object().expect("request object").contains_key(key) {
                return Err(configuration_failure(
                    "request options conflict with connector fields",
                ));
            }
            body[key] = value.clone();
        }
    }
    Ok(body)
}

fn image_message(
    parts: &[yo_core::ModelInputPart],
    policy: Option<&ImageWirePolicy>,
    tokenization: bool,
) -> Result<Value, ConnectorError> {
    if policy.is_none() {
        return Err(configuration_failure(
            "image input requires a reviewed connector image profile",
        ));
    }
    let content = parts.iter().filter_map(|part| match part {
        yo_core::ModelInputPart::Text { text } => Some(json!({"type":"text", "text":text})),
        yo_core::ModelInputPart::Image { .. } if tokenization => None,
        yo_core::ModelInputPart::Image { snapshot } => Some(json!({
            "type":"image_url", "image_url":{"url":format!("data:image/png;base64,{}", STANDARD.encode(snapshot.png()))}
        })),
    }).collect::<Vec<_>>();
    Ok(json!({"role":"user", "content":content}))
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

#[cfg(test)]
mod image_tests;
