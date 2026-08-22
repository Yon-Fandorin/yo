use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};
use yo_connector_transport::{DecodeBatch, SseFrame, SseFramer};
use yo_core::{
    ConnectorError, ConnectorFailureKind, ModelConnectorEvent, ModelConnectorLimits,
    ModelConnectorTerminal, ModelConnectorUsage, ModelReplayBudget, ModelRequestFailureKind,
};

use crate::private_replay::{
    KimiAssistantMessage, KimiAssistantToolCall, KimiReplayToolCallSize,
    kimi_replay_round_item_lengths,
};

const KIMI_PRIVATE_MESSAGE_FIXED_BYTES: usize =
    br#"{"content":,"reasoning_content":"","role":"assistant"}"#.len();
const KIMI_TOOL_CALLS_FIELD_BYTES: usize = 16;

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
    model: String,
    private_replay: bool,
    role_seen: bool,
    content_seen: bool,
    content: String,
    reasoning_content: String,
    private_content_encoded_bytes: usize,
    private_reasoning_encoded_bytes: usize,
    private_tool_calls_encoded_bytes: usize,
    replay_budget: ModelReplayBudget,
}

#[derive(Clone)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
    encoded_size: KimiReplayToolCallSize,
}

impl ChatCompletionsSseDecoder {
    #[cfg(test)]
    pub(super) fn new_kimi(
        limits: ModelConnectorLimits,
        model: String,
        private_replay: bool,
    ) -> Self {
        let replay_budget = yo_core::ModelReplayDelta::replay_budget(None, std::iter::empty())
            .expect("an empty replay delta prefix fits its canonical bound");
        Self::new_kimi_with_replay_budget(limits, model, private_replay, replay_budget)
    }

    pub(super) fn new_kimi_with_replay_budget(
        limits: ModelConnectorLimits,
        model: String,
        private_replay: bool,
        replay_budget: ModelReplayBudget,
    ) -> Self {
        let limits = crate::bounded_kimi_limits(limits);
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
            model,
            private_replay,
            role_seen: false,
            content_seen: false,
            content: String::new(),
            reasoning_content: String::new(),
            private_content_encoded_bytes: 0,
            private_reasoning_encoded_bytes: 0,
            private_tool_calls_encoded_bytes: 0,
            replay_budget,
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

    #[cfg(test)]
    pub(super) fn kimi_private_retained_lengths(&self) -> (usize, usize, usize) {
        (
            self.content.len(),
            self.reasoning_content.len(),
            self.calls.len(),
        )
    }

    #[cfg(test)]
    pub(super) const fn kimi_retained_argument_bytes(&self) -> usize {
        self.argument_bytes
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
        self.validate_kimi_chunk_identity(chunk)?;
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
            self.accept_usage(decode_usage(usage)?)?;
            return Ok(emitted);
        }
        if choices.len() != 1 {
            return Err(protocol_failure(
                "Chat Completions stream must contain exactly one choice",
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
        self.validate_kimi_delta_shape(delta)?;
        let role = optional_string(delta.get("role"), "delta.role")?;
        self.validate_role(role)?;
        if let Some(content) = optional_string(delta.get("content"), "delta.content")? {
            let encoded = encoded_json_string_payload_bytes(content)?;
            let prospective = self
                .private_content_encoded_bytes
                .checked_add(encoded)
                .ok_or_else(|| limit_failure("Kimi private assistant byte limit exceeded"))?;
            if self.private_replay {
                self.ensure_kimi_private_budget(
                    true,
                    prospective,
                    self.private_reasoning_encoded_bytes,
                    self.private_tool_calls_encoded_bytes,
                )?;
            }
            self.ensure_kimi_complete_replay_budget(
                true,
                prospective,
                self.private_reasoning_encoded_bytes,
                None,
            )?;
            self.add_content_bytes(content.len())?;
            self.private_content_encoded_bytes = prospective;
            self.message_seen = true;
            self.content_seen = true;
            self.content.push_str(content);
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
            if !self.private_replay {
                return Err(protocol_failure(
                    "Kimi K2.6 response carried reasoning_content",
                ));
            }
            let encoded = encoded_json_string_payload_bytes(reasoning)?;
            let prospective = self
                .private_reasoning_encoded_bytes
                .checked_add(encoded)
                .ok_or_else(|| limit_failure("Kimi private assistant byte limit exceeded"))?;
            self.ensure_kimi_private_budget(
                self.content_seen,
                self.private_content_encoded_bytes,
                prospective,
                self.private_tool_calls_encoded_bytes,
            )?;
            self.ensure_kimi_complete_replay_budget(
                self.content_seen,
                self.private_content_encoded_bytes,
                prospective,
                None,
            )?;
            self.add_reasoning_bytes(reasoning.len())?;
            self.private_reasoning_encoded_bytes = prospective;
            self.reasoning_content.push_str(reasoning);
        }
        if let Some(tool_calls) = delta.get("tool_calls") {
            let tool_calls = tool_calls.as_array().ok_or_else(|| {
                protocol_failure("Chat Completions delta.tool_calls is not an array")
            })?;
            for fragment in tool_calls {
                self.apply_tool_fragment(fragment)?;
            }
        }

        let finish_reason = optional_string(choice.get("finish_reason"), "finish_reason")?;
        if finish_reason.is_none()
            && (usage.is_some() || choice.get("usage").is_some_and(|usage| !usage.is_null()))
        {
            return Err(protocol_failure(
                "Kimi usage arrived before the finish reason",
            ));
        }
        if let Some(reason) = finish_reason {
            let status = match reason {
                "stop" => {
                    if !self.calls.is_empty() {
                        return Err(protocol_failure(
                            "Chat Completions stop finish contradicts the accumulated round",
                        ));
                    }
                    if !self.content_seen {
                        return Err(protocol_failure(
                            "Kimi stop response requires string assistant content",
                        ));
                    }
                    self.ensure_kimi_complete_replay_budget(
                        self.content_seen,
                        self.private_content_encoded_bytes,
                        self.private_reasoning_encoded_bytes,
                        None,
                    )?;
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
                    self.ensure_kimi_complete_replay_budget(
                        self.content_seen,
                        self.private_content_encoded_bytes,
                        self.private_reasoning_encoded_bytes,
                        None,
                    )?;
                    emitted.push(ModelConnectorEvent::MessageDone {
                        output_index: 0,
                        item_id: self.message_item_id(),
                    });
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
                    self.emit_kimi_private(&mut emitted)?;
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
                        request_failure: ModelRequestFailureKind::ResponseLimit,
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
                        request_failure: ModelRequestFailureKind::RequestRejected,
                    }
                },
                _ => {
                    return Err(protocol_failure(
                        "Chat Completions finish reason is unsupported",
                    ));
                },
            };
            if matches!(reason, "stop") {
                self.emit_kimi_private(&mut emitted)?;
            }
            if let Some(usage) = usage {
                self.accept_usage(decode_usage(usage)?)?;
            }
            if let Some(choice_usage) = choice.get("usage").filter(|usage| !usage.is_null()) {
                self.accept_usage(decode_usage(choice_usage)?)?;
            }
            self.finish = Some(status);
        }
        Ok(emitted)
    }

    fn kimi_private_replay(&self) -> bool {
        self.private_replay
    }

    fn validate_kimi_chunk_identity(&self, chunk: &Value) -> Result<(), ConnectorError> {
        if chunk.get("object").and_then(Value::as_str) != Some("chat.completion.chunk")
            || chunk.get("model").and_then(Value::as_str) != Some(self.model.as_str())
        {
            return Err(protocol_failure(
                "Kimi stream object or model does not match the request",
            ));
        }
        Ok(())
    }

    fn validate_kimi_delta_shape(
        &self,
        delta: &serde_json::Map<String, Value>,
    ) -> Result<(), ConnectorError> {
        const ALLOWED: [&str; 4] = ["role", "content", "reasoning_content", "tool_calls"];
        if delta.keys().any(|field| !ALLOWED.contains(&field.as_str())) {
            return Err(protocol_failure("Kimi delta contains an undeclared field"));
        }
        Ok(())
    }

    fn validate_role(&mut self, role: Option<&str>) -> Result<(), ConnectorError> {
        match (self.role_seen, role) {
            (false, Some("assistant")) => self.role_seen = true,
            (false, _) => {
                return Err(protocol_failure(
                    "the first Kimi choice delta requires role assistant",
                ));
            },
            (true, None) => {},
            (true, Some(_)) => {
                return Err(protocol_failure(
                    "Kimi assistant role was repeated or changed",
                ));
            },
        }
        Ok(())
    }

    fn emit_kimi_private(
        &mut self,
        emitted: &mut Vec<ModelConnectorEvent>,
    ) -> Result<(), ConnectorError> {
        if !self.kimi_private_replay() {
            return Ok(());
        }
        self.ensure_kimi_complete_replay_budget(
            self.content_seen,
            self.private_content_encoded_bytes,
            self.private_reasoning_encoded_bytes,
            None,
        )?;
        let output_index = self.calls.len() + 1;
        let tool_calls = std::mem::take(&mut self.calls)
            .into_values()
            .map(|call| KimiAssistantToolCall::new(call.id, call.name, call.arguments))
            .collect::<Vec<_>>();
        let message = KimiAssistantMessage::new(
            std::mem::take(&mut self.reasoning_content),
            self.content_seen.then(|| std::mem::take(&mut self.content)),
            tool_calls,
        );
        emitted.push(ModelConnectorEvent::ProviderPrivateAssistant {
            output_index,
            envelope: crate::private_replay::encode_envelope(&message)?,
            visible_projection: crate::private_replay::visible_projection(&message),
        });
        Ok(())
    }

    fn accept_usage(&mut self, usage: ModelConnectorUsage) -> Result<(), ConnectorError> {
        if let Some(existing) = &self.usage {
            if existing == &usage {
                return Ok(());
            }
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
            if !(1..=4 * 1024).contains(&id.len()) {
                return Err(protocol_failure(
                    "Kimi tool-call id is outside its exact byte bounds",
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
            if !valid_kimi_function_name(&name) {
                return Err(protocol_failure(
                    "Kimi tool-call function name is outside its closed grammar",
                ));
            }
            let arguments = optional_string(function.get("arguments"), "tool-call arguments")?
                .unwrap_or_default()
                .to_owned();
            let encoded_size = KimiReplayToolCallSize {
                id_json_bytes: encoded_json_string_payload_bytes(&id)?,
                name_json_bytes: encoded_json_string_payload_bytes(&name)?,
                arguments_json_bytes: encoded_json_string_payload_bytes(&arguments)?,
            };
            let prospective_private_tool_bytes = if self.kimi_private_replay() {
                let call_bytes = serde_json::to_vec(&json!({
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments},
                }))
                .map_err(|_| protocol_failure("Kimi private tool call cannot be encoded"))?
                .len();
                let separator = usize::from(self.private_tool_calls_encoded_bytes != 0);
                let prospective = self
                    .private_tool_calls_encoded_bytes
                    .checked_add(separator)
                    .and_then(|total| total.checked_add(call_bytes))
                    .ok_or_else(|| limit_failure("Kimi private assistant byte limit exceeded"))?;
                self.ensure_kimi_private_budget(
                    self.content_seen,
                    self.private_content_encoded_bytes,
                    self.private_reasoning_encoded_bytes,
                    prospective,
                )?;
                Some(prospective)
            } else {
                None
            };
            self.ensure_kimi_complete_replay_budget(
                self.content_seen,
                self.private_content_encoded_bytes,
                self.private_reasoning_encoded_bytes,
                Some((index, encoded_size)),
            )?;
            self.add_argument_bytes(arguments.len())?;
            if let Some(prospective) = prospective_private_tool_bytes {
                self.private_tool_calls_encoded_bytes = prospective;
            }
            self.call_ids.insert(id.clone());
            self.calls.insert(
                index,
                ToolCall {
                    id,
                    name,
                    arguments,
                    encoded_size,
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
        let encoded = encoded_json_string_payload_bytes(arguments)?;
        let prospective_size = KimiReplayToolCallSize {
            arguments_json_bytes: call
                .encoded_size
                .arguments_json_bytes
                .checked_add(encoded)
                .ok_or_else(|| limit_failure("Kimi complete replay byte limit exceeded"))?,
            ..call.encoded_size
        };
        let prospective_private_tool_bytes = if self.kimi_private_replay() {
            let prospective = self
                .private_tool_calls_encoded_bytes
                .checked_add(encoded)
                .ok_or_else(|| limit_failure("Kimi private assistant byte limit exceeded"))?;
            self.ensure_kimi_private_budget(
                self.content_seen,
                self.private_content_encoded_bytes,
                self.private_reasoning_encoded_bytes,
                prospective,
            )?;
            Some(prospective)
        } else {
            None
        };
        self.ensure_kimi_complete_replay_budget(
            self.content_seen,
            self.private_content_encoded_bytes,
            self.private_reasoning_encoded_bytes,
            Some((index, prospective_size)),
        )?;
        self.add_argument_bytes(arguments.len())?;
        if let Some(prospective) = prospective_private_tool_bytes {
            self.private_tool_calls_encoded_bytes = prospective;
        }
        let call = self
            .calls
            .get_mut(&index)
            .expect("an admitted tool-call index exists");
        call.encoded_size = prospective_size;
        call.arguments.push_str(arguments);
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

    fn ensure_kimi_private_budget(
        &self,
        content_seen: bool,
        content_encoded_bytes: usize,
        reasoning_encoded_bytes: usize,
        tool_calls_encoded_bytes: usize,
    ) -> Result<(), ConnectorError> {
        let content_bytes = if content_seen {
            content_encoded_bytes.checked_add(2)
        } else {
            Some(4)
        };
        let mut total = KIMI_PRIVATE_MESSAGE_FIXED_BYTES
            .checked_add(
                content_bytes
                    .ok_or_else(|| limit_failure("Kimi private assistant byte limit exceeded"))?,
            )
            .and_then(|total| total.checked_add(reasoning_encoded_bytes));
        if tool_calls_encoded_bytes != 0 {
            total = total
                .and_then(|total| total.checked_add(KIMI_TOOL_CALLS_FIELD_BYTES))
                .and_then(|total| total.checked_add(tool_calls_encoded_bytes));
        }
        if total.is_none_or(|total| total > self.limits.max_provider_private_bytes) {
            return Err(limit_failure("Kimi private assistant byte limit exceeded"));
        }
        Ok(())
    }

    fn ensure_kimi_complete_replay_budget(
        &self,
        content_seen: bool,
        content_encoded_bytes: usize,
        reasoning_encoded_bytes: usize,
        updated_call: Option<(usize, KimiReplayToolCallSize)>,
    ) -> Result<(), ConnectorError> {
        let replay_budget = self.replay_budget;
        let call_indices = self
            .calls
            .keys()
            .copied()
            .chain(updated_call.map(|(index, _)| index))
            .collect::<std::collections::BTreeSet<_>>();
        let call_sizes = call_indices
            .into_iter()
            .map(|index| {
                updated_call
                    .filter(|(updated_index, _)| *updated_index == index)
                    .map(|(_, size)| size)
                    .or_else(|| self.calls.get(&index).map(|call| call.encoded_size))
                    .expect("every retained Kimi call has an encoded size")
            })
            .collect::<Vec<_>>();
        let item_lengths = kimi_replay_round_item_lengths(
            self.kimi_private_replay(),
            content_seen,
            content_encoded_bytes,
            reasoning_encoded_bytes,
            &call_sizes,
        )
        .ok_or_else(|| limit_failure("Kimi complete replay byte limit exceeded"))?;
        if replay_budget.accepts_item_lengths(&item_lengths) {
            Ok(())
        } else {
            Err(limit_failure("Kimi complete replay byte limit exceeded"))
        }
    }

    fn message_item_id(&self) -> String {
        format!(
            "{}:message",
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

pub(crate) fn decode_usage(value: &Value) -> Result<ModelConnectorUsage, ConnectorError> {
    const CACHE_READ_SOURCE_PROFILE: &str = "kimi.usage.cached-tokens/v1";

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
    let cache_read_input_tokens = match value.get("cached_tokens") {
        Some(cached) => {
            let tokens = cached.as_u64().ok_or_else(|| {
                protocol_failure("Chat Completions cached_tokens is not non-negative")
            })?;
            if tokens > prompt {
                return Err(protocol_failure(
                    "Chat Completions cached_tokens exceeds prompt_tokens",
                ));
            }
            yo_core::CacheReadInputTokens::Reported {
                tokens,
                source_profile: yo_core::VersionedProfileId::new(CACHE_READ_SOURCE_PROFILE)
                    .expect("the closed Kimi usage source profile is valid"),
            }
        },
        None => yo_core::CacheReadInputTokens::Absent {
            source_profile: yo_core::VersionedProfileId::new(CACHE_READ_SOURCE_PROFILE)
                .expect("the closed Kimi usage source profile is valid"),
        },
    };
    Ok(ModelConnectorUsage {
        input_tokens: Some(prompt),
        output_tokens: Some(completion),
        total_tokens: Some(total),
        reasoning_tokens: reasoning,
        cache_read_input_tokens,
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

fn encoded_json_string_payload_bytes(value: &str) -> Result<usize, ConnectorError> {
    serde_json::to_string(value)
        .map(|encoded| encoded.len() - 2)
        .map_err(|_| protocol_failure("Kimi private string cannot be encoded"))
}

fn valid_kimi_function_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    (3..=64).contains(&bytes.len())
        && bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
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
