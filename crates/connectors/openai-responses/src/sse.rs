use std::collections::{HashMap, HashSet};

use serde_json::Value;
use yo_connector_transport::{DecodeBatch as SseDecodeBatch, SseDecoder, SseFrame, SseFramer};
use yo_core::{
    CacheReadInputTokens, ConnectorError, ConnectorFailureKind, ModelConnectorEvent,
    ModelConnectorLimits, ModelConnectorTerminal, ModelConnectorUsage, ReasoningChannel,
    VersionedProfileId,
};

mod delta;
mod item;
mod terminal;
mod wire;

use self::wire::{
    limit_failure, optional_string_at, protocol_failure, responses_failed_failure,
    responses_incomplete_failure, string_at, usage_at, usize_at, value_at,
};

const CACHE_READ_SOURCE_PROFILE: &str =
    "openai.responses.usage.input-tokens-details.cached-tokens/v1";

pub(super) struct ResponsesSseDecoder {
    limits: ModelConnectorLimits,
    framer: SseFramer,
    output_items: HashMap<usize, OutputItem>,
    item_ids: HashSet<String>,
    response_id: Option<String>,
    last_sequence: Option<u64>,
    response_text_bytes: usize,
    function_argument_bytes: usize,
    terminated: bool,
    done_marker_seen: bool,
    pending_terminal: Option<ModelConnectorEvent>,
}

struct OutputItem {
    id: String,
    kind: OutputItemKind,
    done: bool,
}

enum OutputItemKind {
    Message {
        content: HashMap<usize, MessageContent>,
    },
    Reasoning {
        parts: HashMap<ReasoningPart, TextPart>,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
        arguments_done: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ReasoningPart {
    channel: ReasoningChannel,
    index: usize,
}

struct TextPart {
    value: String,
    stream_done: bool,
    declared: bool,
    part_done: bool,
}

enum MessageContent {
    Text {
        value: String,
        stream_done: bool,
        declared: bool,
        part_done: bool,
    },
    Refusal {
        value: String,
        stream_done: bool,
        declared: bool,
        part_done: bool,
    },
}

impl SseDecoder for ResponsesSseDecoder {
    fn push(&mut self, bytes: &[u8]) -> SseDecodeBatch {
        self.push_batch(bytes)
    }
    fn finish(&mut self) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        self.finish()
    }
}

impl ResponsesSseDecoder {
    pub(super) fn new(limits: ModelConnectorLimits) -> Self {
        let framer = SseFramer::new(
            limits.max_sse_event_bytes,
            limits.max_sse_events,
            "Responses",
        );
        Self {
            limits,
            framer,
            output_items: HashMap::new(),
            item_ids: HashSet::new(),
            response_id: None,
            last_sequence: None,
            response_text_bytes: 0,
            function_argument_bytes: 0,
            terminated: false,
            done_marker_seen: false,
            pending_terminal: None,
        }
    }

    fn item_mut(
        &mut self,
        output_index: usize,
        item_id: &str,
    ) -> Result<&mut OutputItem, ConnectorError> {
        let item = self
            .output_items
            .get_mut(&output_index)
            .ok_or_else(|| protocol_failure("event references an unknown output index"))?;
        if item.id != item_id {
            return Err(protocol_failure(
                "event item_id disagrees with its output index",
            ));
        }
        Ok(item)
    }

    fn add_text_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.response_text_bytes = self
            .response_text_bytes
            .checked_add(bytes)
            .ok_or_else(|| limit_failure("response text byte count overflowed"))?;
        if self.response_text_bytes > self.limits.max_response_text_bytes {
            return Err(limit_failure("cumulative response text limit exceeded"));
        }
        Ok(())
    }

    fn add_argument_bytes(&mut self, bytes: usize) -> Result<(), ConnectorError> {
        self.function_argument_bytes = self
            .function_argument_bytes
            .checked_add(bytes)
            .ok_or_else(|| limit_failure("function argument byte count overflowed"))?;
        if self.function_argument_bytes > self.limits.max_function_argument_bytes {
            return Err(limit_failure("cumulative function argument limit exceeded"));
        }
        Ok(())
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

    fn push_batch(&mut self, bytes: &[u8]) -> SseDecodeBatch {
        let mut decoded = Vec::new();
        let frames = self.framer.push(bytes);
        for frame in frames.frames {
            match self.decode_event(frame) {
                Ok(events) => decoded.extend(events),
                Err(failure) => {
                    return SseDecodeBatch {
                        events: decoded,
                        failure: Some(failure),
                    };
                },
            }
        }
        SseDecodeBatch {
            events: decoded,
            failure: frames.failure,
        }
    }

    fn finish(&mut self) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        let mut decoded = Vec::new();
        if let Some(frame) = self.framer.finish()? {
            decoded.extend(self.decode_event(frame)?);
        }
        if !self.terminated {
            return Err(protocol_failure(
                "Responses stream ended without a terminal response event",
            ));
        }
        let terminal = self
            .pending_terminal
            .take()
            .ok_or_else(|| protocol_failure("Responses terminal event was not retained"))?;
        decoded.push(terminal);
        Ok(decoded)
    }

    fn decode_event(
        &mut self,
        frame: SseFrame,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        let Some(data) = frame.data else {
            return Ok(Vec::new());
        };
        if self.terminated {
            if data == "[DONE]" && frame.declared_event.is_none() && !self.done_marker_seen {
                self.done_marker_seen = true;
                return Ok(Vec::new());
            }
            return Err(protocol_failure(
                "Responses stream contained data after its terminal event",
            ));
        }
        if data == "[DONE]" {
            return Err(protocol_failure(
                "Chat Completions [DONE] marker is not a Responses terminal event",
            ));
        }
        let event: Value = serde_json::from_str(&data)
            .map_err(|_| protocol_failure("Responses SSE data is not valid JSON"))?;
        let event_type = string_at(&event, &["type"], "event type")?;
        if frame
            .declared_event
            .is_some_and(|declared| declared != event_type)
        {
            return Err(protocol_failure(
                "Responses SSE event field disagrees with its JSON type",
            ));
        }
        self.observe_sequence(&event)?;
        self.decode_wire_event(event_type, &event)
    }

    fn observe_sequence(&mut self, event: &Value) -> Result<(), ConnectorError> {
        let Some(sequence) = event.get("sequence_number") else {
            return Ok(());
        };
        let sequence = sequence.as_u64().ok_or_else(|| {
            protocol_failure("Responses sequence_number is not an unsigned integer")
        })?;
        if self.last_sequence.is_some_and(|last| sequence <= last) {
            return Err(protocol_failure(
                "Responses sequence_number did not increase",
            ));
        }
        self.last_sequence = Some(sequence);
        Ok(())
    }

    fn decode_wire_event(
        &mut self,
        event_type: &str,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        match event_type {
            "response.created" => self.response_created(event),
            "response.output_item.added" => self.output_item_added(event),
            "response.output_item.done" => self.output_item_done(event),
            "response.content_part.added" => self.content_part_added(event),
            "response.content_part.done" => self.content_part_done(event),
            "response.reasoning_summary_part.added" => self.reasoning_summary_part_added(event),
            "response.reasoning_summary_part.done" => self.reasoning_summary_part_done(event),
            "response.output_text.delta" => self.output_text_delta(event),
            "response.output_text.done" => self.output_text_done(event),
            "response.refusal.delta" => self.refusal_delta(event),
            "response.refusal.done" => self.refusal_done(event),
            "response.reasoning_text.delta" => {
                self.reasoning_delta(event, ReasoningChannel::Text, "content_index")
            },
            "response.reasoning_summary_text.delta" => {
                self.reasoning_delta(event, ReasoningChannel::Summary, "summary_index")
            },
            "response.reasoning_text.done" => {
                self.reasoning_done(event, ReasoningChannel::Text, "content_index")
            },
            "response.reasoning_summary_text.done" => {
                self.reasoning_done(event, ReasoningChannel::Summary, "summary_index")
            },
            "response.function_call_arguments.delta" => self.function_arguments_delta(event),
            "response.function_call_arguments.done" => self.function_arguments_done(event),
            "response.completed" => self.terminal(event, ModelConnectorTerminal::Completed),
            "response.incomplete" => {
                let reason =
                    optional_string_at(event, &["response", "incomplete_details", "reason"])?;
                let request_failure = responses_incomplete_failure(reason.as_deref());
                self.terminal(
                    event,
                    ModelConnectorTerminal::Incomplete {
                        reason,
                        request_failure,
                    },
                )
            },
            "response.failed" => {
                let code = optional_string_at(event, &["response", "error", "code"])?;
                let request_failure = responses_failed_failure(code.as_deref());
                self.terminal(
                    event,
                    ModelConnectorTerminal::Failed {
                        code,
                        request_failure,
                    },
                )
            },
            "response.queued" | "response.in_progress" => Ok(Vec::new()),
            _ => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests;
