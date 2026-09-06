use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};
use yo_connector_transport::{DecodeBatch, SseDecoder, SseFrame, SseFramer};
use yo_core::{
    ConnectorError, ConnectorFailureKind, ModelConnectorEvent, ModelConnectorLimits,
    ModelConnectorTerminal, ModelConnectorUsage, ModelReplayBudget, ModelRequestFailureKind,
};

use crate::private_replay::{
    KimiAssistantMessage, KimiAssistantToolCall, KimiReplayToolCallSize,
    kimi_replay_round_item_lengths,
};

mod budget;
mod private;
mod tool;
mod usage;
mod wire;

use self::{
    usage::decode_usage,
    wire::{
        encoded_json_string_payload_bytes, limit_failure, optional_string, protocol_failure,
        string_at, unsigned_at,
    },
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

impl SseDecoder for ChatCompletionsSseDecoder {
    fn push(&mut self, bytes: &[u8]) -> DecodeBatch {
        self.push_batch(bytes)
    }
    fn finish(&mut self) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        self.finish()
    }
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
            match (self.finish.is_some(), self.usage.is_some()) {
                (false, false) => {
                    return Err(protocol_failure(
                        "Chat Completions [DONE] arrived before finish reason and final usage",
                    ));
                },
                (false, true) => {
                    return Err(protocol_failure(
                        "Chat Completions [DONE] arrived before finish reason",
                    ));
                },
                (true, false) => {
                    return Err(protocol_failure(
                        "Chat Completions [DONE] arrived before final usage",
                    ));
                },
                (true, true) => {},
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
