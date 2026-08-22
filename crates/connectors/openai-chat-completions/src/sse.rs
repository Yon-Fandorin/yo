use std::collections::{BTreeMap, HashSet};

use serde_json::Value;
use yo_connector_transport::{DecodeBatch, SseFrame, SseFramer};
use yo_core::{
    ConnectorError, ConnectorFailureKind, ModelConnectorEvent, ModelConnectorLimits,
    ModelConnectorTerminal, ModelConnectorUsage, ReasoningChannel,
};

pub(super) struct ChatCompletionsSseDecoder {
    limits: ModelConnectorLimits,
    framer: SseFramer,
    response_id: Option<String>,
    message_seen: bool,
    content_bytes: usize,
    refusal_bytes: usize,
    reasoning_bytes: usize,
    argument_bytes: usize,
    calls: BTreeMap<usize, ToolCall>,
    call_ids: HashSet<String>,
    finish: Option<ModelConnectorTerminal>,
    usage: Option<ModelConnectorUsage>,
    done_seen: bool,
}

struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl ChatCompletionsSseDecoder {
    pub(super) fn new(limits: ModelConnectorLimits) -> Self {
        let framer = SseFramer::new(
            limits.max_sse_event_bytes,
            limits.max_sse_events,
            "Chat Completions",
        );
        Self {
            limits,
            framer,
            response_id: None,
            message_seen: false,
            content_bytes: 0,
            refusal_bytes: 0,
            reasoning_bytes: 0,
            argument_bytes: 0,
            calls: BTreeMap::new(),
            call_ids: HashSet::new(),
            finish: None,
            usage: None,
            done_seen: false,
        }
    }

    #[cfg(test)]
    pub(super) fn push(
        &mut self,
        bytes: &[u8],
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        let batch = self.push_batch(bytes);
        match batch.failure {
            Some(failure) => Err(failure),
            None => Ok(batch.events),
        }
    }

    pub(super) fn push_batch(&mut self, bytes: &[u8]) -> DecodeBatch {
        let mut decoded = Vec::new();
        let frames = self.framer.push(bytes);
        for frame in frames.frames {
            match self.decode_event(frame) {
                Ok(events) => decoded.extend(events),
                Err(failure) => {
                    return DecodeBatch {
                        events: decoded,
                        failure: Some(failure),
                    };
                },
            }
        }
        DecodeBatch {
            events: decoded,
            failure: frames.failure,
        }
    }

    pub(super) fn finish(&mut self) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        if let Some(frame) = self.framer.finish()? {
            let emitted = self.decode_event(frame)?;
            if !emitted.is_empty() {
                return Err(protocol_failure(
                    "Chat Completions stream ended while an emitted event remained unterminated",
                ));
            }
        }
        if !self.done_seen {
            return Err(protocol_failure(
                "Chat Completions stream ended without [DONE]",
            ));
        }
        let response_id = self.response_id.clone().ok_or_else(|| {
            protocol_failure("Chat Completions stream has no correlated response id")
        })?;
        let status = self
            .finish
            .clone()
            .ok_or_else(|| protocol_failure("Chat Completions stream has no finish reason"))?;
        let usage = self
            .usage
            .clone()
            .ok_or_else(|| protocol_failure("Chat Completions stream has no final usage record"))?;
        Ok(vec![ModelConnectorEvent::Terminal {
            response_id,
            status,
            usage,
        }])
    }

    fn decode_event(
        &mut self,
        frame: SseFrame,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        let Some(data) = frame.data else {
            return Ok(Vec::new());
        };
        if self.done_seen {
            return Err(protocol_failure(
                "Chat Completions stream contained data after [DONE]",
            ));
        }
        if data == "[DONE]" {
            if frame.declared_event.is_some() {
                return Err(protocol_failure(
                    "Chat Completions [DONE] sentinel must not have an SSE event name",
                ));
            }
            if self.finish.is_none() || self.usage.is_none() {
                return Err(protocol_failure(
                    "Chat Completions [DONE] arrived before finish and final usage",
                ));
            }
            self.done_seen = true;
            return Ok(Vec::new());
        }

        let chunk: Value = serde_json::from_str(&data)
            .map_err(|_| protocol_failure("Chat Completions SSE data is not valid JSON"))?;
        self.decode_chunk(&chunk)
    }

    fn decode_chunk(&mut self, chunk: &Value) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        let id = string_at(chunk, "id", "response id")?;
        let mut emitted = Vec::new();
        match &self.response_id {
            None => {
                self.response_id = Some(id.to_owned());
                emitted.push(ModelConnectorEvent::ResponseCreated {
                    response_id: id.to_owned(),
                });
            },
            Some(existing) if existing != id => {
                return Err(protocol_failure(
                    "Chat Completions response id changed during the stream",
                ));
            },
            Some(_) => {},
        }

        let choices = chunk
            .get("choices")
            .and_then(Value::as_array)
            .ok_or_else(|| protocol_failure("Chat Completions choices is not an array"))?;
        let usage = chunk.get("usage").filter(|usage| !usage.is_null());
        if choices.is_empty() {
            let usage = usage.ok_or_else(|| {
                protocol_failure("empty Chat Completions choices requires final usage")
            })?;
            if self.finish.is_none() {
                return Err(protocol_failure(
                    "Chat Completions usage arrived before the finish reason",
                ));
            }
            if self.usage.is_some() {
                return Err(protocol_failure(
                    "duplicate Chat Completions final usage record",
                ));
            }
            self.accept_usage(decode_usage(usage)?)?;
            return Ok(emitted);
        }
        if choices.len() != 1 {
            return Err(protocol_failure(
                "Chat Completions stream must contain exactly one choice",
            ));
        }
        if usage.is_some() {
            return Err(protocol_failure(
                "non-null Chat Completions usage appeared on a choice chunk",
            ));
        }
        if self.finish.is_some() {
            return Err(protocol_failure(
                "Chat Completions choice data appeared after its finish reason",
            ));
        }

        let choice = &choices[0];
        if unsigned_at(choice, "index", "choice index")? != 0 {
            return Err(protocol_failure(
                "Chat Completions choice index must be zero",
            ));
        }
        let delta = choice
            .get("delta")
            .and_then(Value::as_object)
            .ok_or_else(|| protocol_failure("Chat Completions delta is not an object"))?;
        let role = optional_string(delta.get("role"), "delta.role")?;
        if role.is_some_and(|role| role != "assistant") {
            return Err(protocol_failure(
                "Chat Completions delta role is not assistant",
            ));
        }
        if let Some(content) = optional_string(delta.get("content"), "delta.content")? {
            self.add_content_bytes(content.len())?;
            self.message_seen = true;
            emitted.push(ModelConnectorEvent::TextDelta {
                output_index: 0,
                item_id: self.message_item_id(),
                content_index: 0,
                delta: content.to_owned(),
            });
        }
        if let Some(refusal) = optional_string(delta.get("refusal"), "delta.refusal")? {
            self.add_refusal_bytes(refusal.len())?;
            self.message_seen = true;
            emitted.push(ModelConnectorEvent::RefusalDelta {
                output_index: 0,
                item_id: self.message_item_id(),
                content_index: 1,
                delta: refusal.to_owned(),
            });
        }
        if let Some(reasoning) =
            optional_string(delta.get("reasoning_content"), "delta.reasoning_content")?
        {
            self.add_reasoning_bytes(reasoning.len())?;
            emitted.push(ModelConnectorEvent::ReasoningDelta {
                output_index: 0,
                item_id: self.reasoning_item_id(),
                channel: ReasoningChannel::Summary,
                part_index: 0,
                delta: reasoning.to_owned(),
            });
        }
        if let Some(tool_calls) = delta.get("tool_calls") {
            let tool_calls = tool_calls.as_array().ok_or_else(|| {
                protocol_failure("Chat Completions delta.tool_calls is not an array")
            })?;
            for fragment in tool_calls {
                self.apply_tool_fragment(fragment)?;
            }
        }

        if let Some(reason) = optional_string(choice.get("finish_reason"), "finish_reason")? {
            let status = match reason {
                "stop" => {
                    if !self.calls.is_empty() {
                        return Err(protocol_failure(
                            "Chat Completions stop finish contradicts the accumulated round",
                        ));
                    }
                    emitted.push(ModelConnectorEvent::MessageDone {
                        output_index: 0,
                        item_id: self.message_item_id(),
                    });
                    ModelConnectorTerminal::Completed
                },
                "tool_calls" => {
                    if self.calls.is_empty() {
                        return Err(protocol_failure(
                            "Chat Completions tool_calls finish has no accumulated tool call",
                        ));
                    }
                    if self.message_seen {
                        emitted.push(ModelConnectorEvent::MessageDone {
                            output_index: 0,
                            item_id: self.message_item_id(),
                        });
                    }
                    for (index, call) in &self.calls {
                        let output_index = index + 1;
                        let item_id = self.call_item_id(*index);
                        emitted.push(ModelConnectorEvent::FunctionCallStarted {
                            output_index,
                            item_id: item_id.clone(),
                            call_id: call.id.clone(),
                            name: call.name.clone(),
                        });
                        emitted.push(ModelConnectorEvent::FunctionCallDone {
                            output_index,
                            item_id,
                            call_id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        });
                    }
                    ModelConnectorTerminal::Completed
                },
                "length" => {
                    if self.message_seen {
                        emitted.push(ModelConnectorEvent::MessageDone {
                            output_index: 0,
                            item_id: self.message_item_id(),
                        });
                    }
                    ModelConnectorTerminal::Incomplete {
                        reason: Some("length".to_owned()),
                        request_failure: yo_core::ModelRequestFailureKind::ResponseLimit,
                    }
                },
                "content_filter" => {
                    if self.message_seen {
                        emitted.push(ModelConnectorEvent::MessageDone {
                            output_index: 0,
                            item_id: self.message_item_id(),
                        });
                    }
                    ModelConnectorTerminal::Failed {
                        code: Some("content_filter".to_owned()),
                        request_failure: yo_core::ModelRequestFailureKind::RequestRejected,
                    }
                },
                _ => {
                    return Err(protocol_failure(
                        "Chat Completions finish reason is unsupported",
                    ));
                },
            };
            self.finish = Some(status);
        }
        Ok(emitted)
    }

    fn accept_usage(&mut self, usage: ModelConnectorUsage) -> Result<(), ConnectorError> {
        if self.usage.is_some() {
            return Err(protocol_failure(
                "duplicate or inconsistent Chat Completions final usage record",
            ));
        }
        self.usage = Some(usage);
        Ok(())
    }

    fn apply_tool_fragment(&mut self, fragment: &Value) -> Result<(), ConnectorError> {
        let index = usize::try_from(unsigned_at(fragment, "index", "tool-call index")?)
            .map_err(|_| protocol_failure("Chat Completions tool-call index is too large"))?;
        if index > self.calls.len() {
            return Err(protocol_failure(
                "Chat Completions tool-call indexes were not introduced contiguously",
            ));
        }
        let function = fragment
            .get("function")
            .and_then(Value::as_object)
            .ok_or_else(|| protocol_failure("Chat Completions tool-call function is missing"))?;
        if let Some(kind) = optional_string(fragment.get("type"), "tool-call type")?
            && kind != "function"
        {
            return Err(protocol_failure(
                "Chat Completions tool-call type is not function",
            ));
        }
        if index == self.calls.len() {
            if self.calls.len() >= self.limits.max_output_items {
                return Err(limit_failure(
                    "Chat Completions tool-call count limit exceeded",
                ));
            }
            let id = string_at(fragment, "id", "tool-call id")?.to_owned();
            if id.is_empty() {
                return Err(protocol_failure(
                    "Chat Completions initial tool-call id is empty",
                ));
            }
            if self.call_ids.contains(&id) {
                return Err(protocol_failure("duplicate Chat Completions tool-call id"));
            }
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    protocol_failure("Chat Completions tool-call function name is missing")
                })?
                .to_owned();
            let arguments = optional_string(function.get("arguments"), "tool-call arguments")?
                .unwrap_or_default()
                .to_owned();
            self.add_argument_bytes(arguments.len())?;
            self.call_ids.insert(id.clone());
            self.calls.insert(
                index,
                ToolCall {
                    id,
                    name,
                    arguments,
                },
            );
            return Ok(());
        }

        let repeated_id = optional_string(fragment.get("id"), "tool-call id")?;
        let repeated_name = optional_string(function.get("name"), "tool-call function name")?;
        let arguments =
            optional_string(function.get("arguments"), "tool-call arguments")?.unwrap_or_default();
        let call = self
            .calls
            .get(&index)
            .expect("an admitted tool-call index exists");
        if repeated_id.is_some_and(|id| !id.is_empty() && id != call.id)
            || repeated_name.is_some_and(|name| name != call.name)
        {
            return Err(protocol_failure(
                "Chat Completions tool-call identity changed across fragments",
            ));
        }
        self.add_argument_bytes(arguments.len())?;
        self.calls
            .get_mut(&index)
            .expect("an admitted tool-call index exists")
            .arguments
            .push_str(arguments);
        Ok(())
    }

    fn add_content_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.content_bytes = bounded_sum(
            self.content_bytes,
            bytes,
            self.limits.max_response_text_bytes,
            "Chat Completions content byte limit exceeded",
        )?;
        Ok(())
    }

    fn add_refusal_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.refusal_bytes = bounded_sum(
            self.refusal_bytes,
            bytes,
            self.limits.max_refusal_bytes,
            "Chat Completions refusal byte limit exceeded",
        )?;
        Ok(())
    }

    fn add_reasoning_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.reasoning_bytes = bounded_sum(
            self.reasoning_bytes,
            bytes,
            self.limits.max_reasoning_bytes,
            "Chat Completions reasoning byte limit exceeded",
        )?;
        Ok(())
    }

    fn add_argument_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.argument_bytes = bounded_sum(
            self.argument_bytes,
            bytes,
            self.limits.max_function_argument_bytes,
            "Chat Completions function-argument byte limit exceeded",
        )?;
        Ok(())
    }

    fn message_item_id(&self) -> String {
        format!(
            "{}:message",
            self.response_id.as_deref().unwrap_or("chat-completion")
        )
    }

    fn reasoning_item_id(&self) -> String {
        format!(
            "{}:reasoning",
            self.response_id.as_deref().unwrap_or("chat-completion")
        )
    }

    fn call_item_id(&self, index: usize) -> String {
        format!(
            "{}:tool-call:{index}",
            self.response_id.as_deref().unwrap_or("chat-completion")
        )
    }
}

fn decode_usage(value: &Value) -> Result<ModelConnectorUsage, ConnectorError> {
    let prompt = non_negative_at(value, "prompt_tokens")?;
    let completion = non_negative_at(value, "completion_tokens")?;
    let total = non_negative_at(value, "total_tokens")?;
    if prompt.checked_add(completion) != Some(total) {
        return Err(protocol_failure(
            "Chat Completions usage total is inconsistent",
        ));
    }
    let reasoning = value
        .get("completion_tokens_details")
        .filter(|details| !details.is_null())
        .and_then(|details| details.get("reasoning_tokens"))
        .filter(|reasoning| !reasoning.is_null())
        .map(|reasoning| {
            reasoning.as_u64().ok_or_else(|| {
                protocol_failure("Chat Completions reasoning_tokens is not non-negative")
            })
        })
        .transpose()?;
    Ok(ModelConnectorUsage {
        input_tokens: Some(prompt),
        output_tokens: Some(completion),
        total_tokens: Some(total),
        reasoning_tokens: reasoning,
        cache_read_input_tokens: yo_core::CacheReadInputTokens::Unsupported,
    })
}

fn non_negative_at(value: &Value, field: &'static str) -> Result<u64, ConnectorError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| protocol_failure(format!("Chat Completions {field} is not non-negative")))
}

fn string_at<'a>(
    value: &'a Value,
    field: &'static str,
    label: &'static str,
) -> Result<&'a str, ConnectorError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| protocol_failure(format!("Chat Completions {label} is missing")))
}

fn unsigned_at(
    value: &Value,
    field: &'static str,
    label: &'static str,
) -> Result<u64, ConnectorError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| protocol_failure(format!("Chat Completions {label} is not unsigned")))
}

fn optional_string<'a>(
    value: Option<&'a Value>,
    label: &'static str,
) -> Result<Option<&'a str>, ConnectorError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(protocol_failure(format!(
            "Chat Completions {label} is not a string or null"
        ))),
    }
}

fn bounded_sum(
    current: usize,
    added: usize,
    limit: usize,
    message: &'static str,
) -> Result<usize, ConnectorError> {
    let total = current
        .checked_add(added)
        .ok_or_else(|| limit_failure(message))?;
    if total > limit {
        Err(limit_failure(message))
    } else {
        Ok(total)
    }
}

fn protocol_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Protocol, message)
}

fn limit_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Limit, message)
}

#[cfg(test)]
mod tests;
