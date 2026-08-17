use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::{
    ConnectorError, ConnectorFailureKind, ReasoningChannel, ResponseTerminal,
    ResponsesConnectorLimits, ResponsesEvent, ResponsesUsage, SseDecodeBatch,
    framing::{SseFrame, SseFramer},
};

pub(super) struct ResponsesSseDecoder {
    limits: ResponsesConnectorLimits,
    framer: SseFramer,
    output_items: HashMap<usize, OutputItem>,
    item_ids: HashSet<String>,
    response_id: Option<String>,
    last_sequence: Option<u64>,
    response_text_bytes: usize,
    function_argument_bytes: usize,
    terminated: bool,
    done_marker_seen: bool,
    pending_terminal: Option<ResponsesEvent>,
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

impl ResponsesSseDecoder {
    pub(super) fn new(limits: ResponsesConnectorLimits) -> Self {
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

    #[cfg(test)]
    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let batch = self.push_batch(bytes);
        match batch.failure {
            Some(failure) => Err(failure),
            None => Ok(batch.events),
        }
    }

    pub(super) fn push_batch(&mut self, bytes: &[u8]) -> SseDecodeBatch {
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

    pub(super) fn finish(&mut self) -> Result<Vec<ResponsesEvent>, ConnectorError> {
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

    fn decode_event(&mut self, frame: SseFrame) -> Result<Vec<ResponsesEvent>, ConnectorError> {
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
    ) -> Result<Vec<ResponsesEvent>, ConnectorError> {
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
            "response.completed" => self.terminal(event, ResponseTerminal::Completed),
            "response.incomplete" => {
                let reason =
                    optional_string_at(event, &["response", "incomplete_details", "reason"])?;
                let request_failure = responses_incomplete_failure(reason.as_deref());
                self.terminal(
                    event,
                    ResponseTerminal::Incomplete {
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
                    ResponseTerminal::Failed {
                        code,
                        request_failure,
                    },
                )
            },
            "response.queued" | "response.in_progress" => Ok(Vec::new()),
            _ => Ok(Vec::new()),
        }
    }

    fn response_created(&mut self, event: &Value) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let response_id = string_at(event, &["response", "id"], "response id")?.to_owned();
        if self.response_id.replace(response_id.clone()).is_some() {
            return Err(protocol_failure("duplicate response.created event"));
        }
        Ok(vec![ResponsesEvent::ResponseCreated { response_id }])
    }

    fn output_item_added(&mut self, event: &Value) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        if self.output_items.len() >= self.limits.max_output_items {
            return Err(limit_failure("Responses output item limit exceeded"));
        }
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let item = value_at(event, &["item"], "output item")?;
        let id = string_at(item, &["id"], "output item id")?.to_owned();
        if self.output_items.contains_key(&output_index) || !self.item_ids.insert(id.clone()) {
            return Err(protocol_failure("duplicate Responses output item identity"));
        }
        let item_type = string_at(item, &["type"], "output item type")?;
        let (kind, emitted) = match item_type {
            "message" => (
                OutputItemKind::Message {
                    content: HashMap::new(),
                },
                Vec::new(),
            ),
            "reasoning" => (
                OutputItemKind::Reasoning {
                    parts: HashMap::new(),
                },
                Vec::new(),
            ),
            "function_call" => {
                let call_id = string_at(item, &["call_id"], "function call_id")?.to_owned();
                let name = string_at(item, &["name"], "function name")?.to_owned();
                let arguments = string_at(item, &["arguments"], "function arguments")?.to_owned();
                self.add_argument_bytes(arguments.len())?;
                let emitted = vec![ResponsesEvent::FunctionCallStarted {
                    output_index,
                    item_id: id.clone(),
                    call_id: call_id.clone(),
                    name: name.clone(),
                }];
                (
                    OutputItemKind::FunctionCall {
                        call_id,
                        name,
                        arguments,
                        arguments_done: false,
                    },
                    emitted,
                )
            },
            _ => {
                return Err(protocol_failure(
                    "Responses stream contained an unsupported output item type",
                ));
            },
        };
        self.output_items.insert(
            output_index,
            OutputItem {
                id,
                kind,
                done: false,
            },
        );
        Ok(emitted)
    }

    fn output_item_done(&mut self, event: &Value) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let item = value_at(event, &["item"], "output item")?;
        let id = string_at(item, &["id"], "output item id")?;
        let item_type = string_at(item, &["type"], "output item type")?;
        let output = self.item_mut(output_index, id)?;
        if output.done {
            return Err(protocol_failure(
                "duplicate response.output_item.done event",
            ));
        }
        let emitted = match (&mut output.kind, item_type) {
            (OutputItemKind::Message { content }, "message") => {
                if content.values().any(|part| !part.is_done()) {
                    return Err(protocol_failure(
                        "message item completed before every content completion event",
                    ));
                }
                vec![ResponsesEvent::MessageDone {
                    output_index,
                    item_id: id.to_owned(),
                }]
            },
            (OutputItemKind::Reasoning { parts }, "reasoning") => {
                if parts.values().any(|part| !part.is_done()) {
                    return Err(protocol_failure(
                        "reasoning item completed before every part completion event",
                    ));
                }
                Vec::new()
            },
            (
                OutputItemKind::FunctionCall {
                    call_id,
                    name,
                    arguments,
                    arguments_done,
                },
                "function_call",
            ) => {
                let final_call_id = string_at(item, &["call_id"], "function call_id")?;
                let final_name = string_at(item, &["name"], "function name")?;
                let final_arguments = string_at(item, &["arguments"], "function arguments")?;
                if final_call_id != call_id || final_name != name || final_arguments != arguments {
                    return Err(protocol_failure(
                        "function output item disagrees with accumulated correlation",
                    ));
                }
                if *arguments_done {
                    Vec::new()
                } else {
                    *arguments_done = true;
                    vec![ResponsesEvent::FunctionCallDone {
                        output_index,
                        item_id: id.to_owned(),
                        call_id: call_id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    }]
                }
            },
            _ => {
                return Err(protocol_failure(
                    "response.output_item.done changed the output item type",
                ));
            },
        };
        output.done = true;
        Ok(emitted)
    }

    fn content_part_added(&mut self, event: &Value) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let content_index = usize_at(event, &["content_index"], "content index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?;
        let part = value_at(event, &["part"], "content part")?;
        let part_type = string_at(part, &["type"], "content part type")?;
        let initial = match part_type {
            "output_text" => string_at(part, &["text"], "initial output text")?,
            "refusal" => string_at(part, &["refusal"], "initial refusal")?,
            "reasoning_text" => string_at(part, &["text"], "initial reasoning text")?,
            _ => {
                return Err(protocol_failure(
                    "Responses stream contained an unsupported content part type",
                ));
            },
        };
        self.add_text_bytes(initial.len())?;
        let output = self.item_mut(output_index, item_id)?;
        if output.done {
            return Err(protocol_failure("late Responses content part"));
        }
        let value = initial.to_owned();
        match (&mut output.kind, part_type) {
            (OutputItemKind::Message { content }, "output_text" | "refusal") => {
                if content.contains_key(&content_index) {
                    return Err(protocol_failure("duplicate Responses message content part"));
                }
                let part = match part_type {
                    "output_text" => MessageContent::Text {
                        value,
                        stream_done: false,
                        declared: true,
                        part_done: false,
                    },
                    "refusal" => MessageContent::Refusal {
                        value,
                        stream_done: false,
                        declared: true,
                        part_done: false,
                    },
                    _ => unreachable!("the message content part type was matched above"),
                };
                content.insert(content_index, part);
            },
            (OutputItemKind::Reasoning { parts }, "reasoning_text") => {
                if parts
                    .insert(
                        ReasoningPart {
                            channel: ReasoningChannel::Text,
                            index: content_index,
                        },
                        TextPart::declared(value),
                    )
                    .is_some()
                {
                    return Err(protocol_failure(
                        "duplicate Responses reasoning content part",
                    ));
                }
            },
            _ => {
                return Err(protocol_failure(
                    "content part type does not correlate to its output item",
                ));
            },
        }
        Ok(Vec::new())
    }

    fn content_part_done(&mut self, event: &Value) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let content_index = usize_at(event, &["content_index"], "content index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?;
        let part = value_at(event, &["part"], "content part")?;
        let part_type = string_at(part, &["type"], "content part type")?;
        let final_value = match part_type {
            "output_text" => string_at(part, &["text"], "final output text")?,
            "refusal" => string_at(part, &["refusal"], "final refusal")?,
            "reasoning_text" => string_at(part, &["text"], "final reasoning text")?,
            _ => {
                return Err(protocol_failure(
                    "Responses stream completed an unsupported content part type",
                ));
            },
        };
        let output = self.item_mut(output_index, item_id)?;
        match (&mut output.kind, part_type) {
            (OutputItemKind::Message { content }, "output_text" | "refusal") => {
                let Some(content) = content.get_mut(&content_index) else {
                    return Err(protocol_failure(
                        "content completion references an unknown message content index",
                    ));
                };
                if !content.matches_type(part_type)
                    || !content.stream_done()
                    || !content.declared()
                    || content.part_done()
                    || content.value() != final_value
                {
                    return Err(protocol_failure(
                        "final message content part disagrees with its accumulated stream",
                    ));
                }
                content.mark_part_done();
            },
            (OutputItemKind::Reasoning { parts }, "reasoning_text") => {
                let Some(part) = parts.get_mut(&ReasoningPart {
                    channel: ReasoningChannel::Text,
                    index: content_index,
                }) else {
                    return Err(protocol_failure(
                        "reasoning content completion references an unknown part",
                    ));
                };
                part.finish_wrapper(final_value)?;
            },
            _ => {
                return Err(protocol_failure(
                    "content completion type does not correlate to its output item",
                ));
            },
        }
        Ok(Vec::new())
    }

    fn reasoning_summary_part_added(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let summary_index = usize_at(event, &["summary_index"], "reasoning summary index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?;
        let part = value_at(event, &["part"], "reasoning summary part")?;
        if string_at(part, &["type"], "reasoning summary part type")? != "summary_text" {
            return Err(protocol_failure(
                "Responses stream contained an unsupported reasoning summary part type",
            ));
        }
        let initial = string_at(part, &["text"], "initial reasoning summary text")?;
        self.add_text_bytes(initial.len())?;
        let output = self.item_mut(output_index, item_id)?;
        let OutputItemKind::Reasoning { parts } = &mut output.kind else {
            return Err(protocol_failure(
                "reasoning summary part does not correlate to a reasoning item",
            ));
        };
        if output.done
            || parts
                .insert(
                    ReasoningPart {
                        channel: ReasoningChannel::Summary,
                        index: summary_index,
                    },
                    TextPart::declared(initial.to_owned()),
                )
                .is_some()
        {
            return Err(protocol_failure(
                "duplicate or late Responses reasoning summary part",
            ));
        }
        Ok(Vec::new())
    }

    fn reasoning_summary_part_done(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let summary_index = usize_at(event, &["summary_index"], "reasoning summary index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?;
        let wrapper = value_at(event, &["part"], "reasoning summary part")?;
        if string_at(wrapper, &["type"], "reasoning summary part type")? != "summary_text" {
            return Err(protocol_failure(
                "Responses stream completed an unsupported reasoning summary part type",
            ));
        }
        let final_text = string_at(wrapper, &["text"], "final reasoning summary text")?;
        let output = self.item_mut(output_index, item_id)?;
        let OutputItemKind::Reasoning { parts } = &mut output.kind else {
            return Err(protocol_failure(
                "reasoning summary completion does not correlate to a reasoning item",
            ));
        };
        let Some(part) = parts.get_mut(&ReasoningPart {
            channel: ReasoningChannel::Summary,
            index: summary_index,
        }) else {
            return Err(protocol_failure(
                "reasoning summary completion references an unknown part",
            ));
        };
        part.finish_wrapper(final_text)?;
        Ok(Vec::new())
    }

    fn output_text_delta(&mut self, event: &Value) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let content_index = usize_at(event, &["content_index"], "content index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?.to_owned();
        let delta = string_at(event, &["delta"], "output text delta")?.to_owned();
        self.add_text_bytes(delta.len())?;
        let output = self.item_mut(output_index, &item_id)?;
        let OutputItemKind::Message { content } = &mut output.kind else {
            return Err(protocol_failure(
                "output text delta does not correlate to a message item",
            ));
        };
        if output.done {
            return Err(protocol_failure(
                "output text arrived after item completion",
            ));
        }
        let part = content
            .entry(content_index)
            .or_insert_with(|| MessageContent::Text {
                value: String::new(),
                stream_done: false,
                declared: false,
                part_done: false,
            });
        let MessageContent::Text {
            value, stream_done, ..
        } = part
        else {
            return Err(protocol_failure(
                "content index changed from refusal to output text",
            ));
        };
        if *stream_done {
            return Err(protocol_failure(
                "output text arrived after content completion",
            ));
        }
        value.push_str(&delta);
        Ok(vec![ResponsesEvent::TextDelta {
            output_index,
            item_id,
            content_index,
            delta,
        }])
    }

    fn output_text_done(&mut self, event: &Value) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let content_index = usize_at(event, &["content_index"], "content index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?;
        let final_text = string_at(event, &["text"], "final output text")?;
        let output = self.item_mut(output_index, item_id)?;
        let OutputItemKind::Message { content } = &mut output.kind else {
            return Err(protocol_failure(
                "output text completion does not correlate to a message item",
            ));
        };
        let Some(MessageContent::Text {
            value, stream_done, ..
        }) = content.get_mut(&content_index)
        else {
            return Err(protocol_failure(
                "output text completion references an unknown text content index",
            ));
        };
        if *stream_done || final_text != value {
            return Err(protocol_failure(
                "final output text disagrees with accumulated deltas",
            ));
        }
        *stream_done = true;
        Ok(Vec::new())
    }

    fn refusal_delta(&mut self, event: &Value) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let content_index = usize_at(event, &["content_index"], "content index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?.to_owned();
        let delta = string_at(event, &["delta"], "refusal delta")?.to_owned();
        self.add_text_bytes(delta.len())?;
        let output = self.item_mut(output_index, &item_id)?;
        let OutputItemKind::Message { content } = &mut output.kind else {
            return Err(protocol_failure(
                "refusal delta does not correlate to a message item",
            ));
        };
        if output.done {
            return Err(protocol_failure("refusal arrived after item completion"));
        }
        let part = content
            .entry(content_index)
            .or_insert_with(|| MessageContent::Refusal {
                value: String::new(),
                stream_done: false,
                declared: false,
                part_done: false,
            });
        let MessageContent::Refusal {
            value, stream_done, ..
        } = part
        else {
            return Err(protocol_failure(
                "content index changed from output text to refusal",
            ));
        };
        if *stream_done {
            return Err(protocol_failure("refusal arrived after content completion"));
        }
        value.push_str(&delta);
        Ok(vec![ResponsesEvent::RefusalDelta {
            output_index,
            item_id,
            content_index,
            delta,
        }])
    }

    fn refusal_done(&mut self, event: &Value) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let content_index = usize_at(event, &["content_index"], "content index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?;
        let final_refusal = string_at(event, &["refusal"], "final refusal")?;
        let output = self.item_mut(output_index, item_id)?;
        let OutputItemKind::Message { content } = &mut output.kind else {
            return Err(protocol_failure(
                "refusal completion does not correlate to a message item",
            ));
        };
        let Some(MessageContent::Refusal {
            value, stream_done, ..
        }) = content.get_mut(&content_index)
        else {
            return Err(protocol_failure(
                "refusal completion references an unknown refusal content index",
            ));
        };
        if *stream_done || final_refusal != value {
            return Err(protocol_failure(
                "final refusal disagrees with accumulated deltas",
            ));
        }
        *stream_done = true;
        Ok(Vec::new())
    }

    fn reasoning_delta(
        &mut self,
        event: &Value,
        channel: ReasoningChannel,
        index_field: &'static str,
    ) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let part_index = usize_at(event, &[index_field], "reasoning part index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?.to_owned();
        let delta = string_at(event, &["delta"], "reasoning delta")?.to_owned();
        self.add_text_bytes(delta.len())?;
        let output = self.item_mut(output_index, &item_id)?;
        let OutputItemKind::Reasoning { parts } = &mut output.kind else {
            return Err(protocol_failure(
                "reasoning delta does not correlate to an active reasoning item",
            ));
        };
        if output.done {
            return Err(protocol_failure(
                "reasoning delta arrived after item completion",
            ));
        }
        let part = parts
            .entry(ReasoningPart {
                channel,
                index: part_index,
            })
            .or_insert_with(TextPart::implicit);
        if part.stream_done {
            return Err(protocol_failure(
                "reasoning delta arrived after part completion",
            ));
        }
        part.value.push_str(&delta);
        Ok(vec![ResponsesEvent::ReasoningDelta {
            output_index,
            item_id,
            channel,
            part_index,
            delta,
        }])
    }

    fn reasoning_done(
        &mut self,
        event: &Value,
        channel: ReasoningChannel,
        index_field: &'static str,
    ) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let part_index = usize_at(event, &[index_field], "reasoning part index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?;
        let final_text = string_at(event, &["text"], "final reasoning text")?;
        let output = self.item_mut(output_index, item_id)?;
        let OutputItemKind::Reasoning { parts } = &mut output.kind else {
            return Err(protocol_failure(
                "reasoning completion does not correlate to a reasoning item",
            ));
        };
        let Some(part) = parts.get_mut(&ReasoningPart {
            channel,
            index: part_index,
        }) else {
            return Err(protocol_failure(
                "reasoning completion references an unknown part",
            ));
        };
        if part.stream_done || final_text != part.value {
            return Err(protocol_failure(
                "final reasoning text disagrees with accumulated deltas",
            ));
        }
        part.stream_done = true;
        Ok(Vec::new())
    }

    fn function_arguments_delta(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?.to_owned();
        let delta = string_at(event, &["delta"], "function arguments delta")?.to_owned();
        self.add_argument_bytes(delta.len())?;
        let output = self.item_mut(output_index, &item_id)?;
        let OutputItemKind::FunctionCall {
            arguments,
            arguments_done,
            ..
        } = &mut output.kind
        else {
            return Err(protocol_failure(
                "function arguments delta does not correlate to a function call",
            ));
        };
        if output.done || *arguments_done {
            return Err(protocol_failure(
                "function arguments arrived after item completion",
            ));
        }
        arguments.push_str(&delta);
        Ok(vec![ResponsesEvent::FunctionArgumentsDelta {
            output_index,
            item_id,
            delta,
        }])
    }

    fn function_arguments_done(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        let output_index = usize_at(event, &["output_index"], "output index")?;
        let item_id = string_at(event, &["item_id"], "output item id")?.to_owned();
        let final_name = string_at(event, &["name"], "function name")?;
        let final_arguments = string_at(event, &["arguments"], "function arguments")?;
        let output = self.item_mut(output_index, &item_id)?;
        let OutputItemKind::FunctionCall {
            call_id,
            name,
            arguments,
            arguments_done,
        } = &mut output.kind
        else {
            return Err(protocol_failure(
                "function arguments completion does not correlate to a function call",
            ));
        };
        if *arguments_done || final_name != name || final_arguments != arguments {
            return Err(protocol_failure(
                "final function arguments disagree with accumulated deltas",
            ));
        }
        *arguments_done = true;
        Ok(vec![ResponsesEvent::FunctionCallDone {
            output_index,
            item_id,
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        }])
    }

    fn terminal(
        &mut self,
        event: &Value,
        status: ResponseTerminal,
    ) -> Result<Vec<ResponsesEvent>, ConnectorError> {
        if self.terminated {
            return Err(protocol_failure("duplicate Responses terminal event"));
        }
        if self.output_items.values().any(|item| !item.done) {
            return Err(protocol_failure(
                "Responses terminal event arrived before every output item completed",
            ));
        }
        let response_id = string_at(event, &["response", "id"], "response id")?.to_owned();
        if self
            .response_id
            .as_ref()
            .is_some_and(|created| created != &response_id)
        {
            return Err(protocol_failure(
                "terminal response id disagrees with response.created",
            ));
        }
        let response_status = string_at(event, &["response", "status"], "response status")?;
        let expected_status = match status {
            ResponseTerminal::Completed => "completed",
            ResponseTerminal::Incomplete { .. } => "incomplete",
            ResponseTerminal::Failed { .. } => "failed",
        };
        if response_status != expected_status {
            return Err(protocol_failure(
                "terminal event type disagrees with response status",
            ));
        }
        let usage = usage_at(event)?;
        self.terminated = true;
        self.pending_terminal = Some(ResponsesEvent::Terminal {
            response_id,
            status,
            usage,
        });
        Ok(Vec::new())
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
}

fn responses_incomplete_failure(reason: Option<&str>) -> crate::ModelRequestFailureKind {
    match reason {
        Some("max_output_tokens") => crate::ModelRequestFailureKind::ResponseLimit,
        Some("content_filter") => crate::ModelRequestFailureKind::RequestRejected,
        _ => crate::ModelRequestFailureKind::Protocol,
    }
}

fn responses_failed_failure(code: Option<&str>) -> crate::ModelRequestFailureKind {
    match code {
        Some("invalid_api_key") => crate::ModelRequestFailureKind::Authentication,
        Some("insufficient_permissions") => crate::ModelRequestFailureKind::AccessDenied,
        Some("model_not_found") => crate::ModelRequestFailureKind::ModelUnavailable,
        Some("rate_limit_exceeded") => crate::ModelRequestFailureKind::RateLimited,
        Some("server_error") => crate::ModelRequestFailureKind::ProviderUnavailable,
        Some("request_timeout") => crate::ModelRequestFailureKind::Timeout,
        _ => crate::ModelRequestFailureKind::Protocol,
    }
}

impl MessageContent {
    fn is_done(&self) -> bool {
        match self {
            Self::Text {
                stream_done,
                declared,
                part_done,
                ..
            }
            | Self::Refusal {
                stream_done,
                declared,
                part_done,
                ..
            } => *stream_done && (!*declared || *part_done),
        }
    }

    fn value(&self) -> &str {
        match self {
            Self::Text { value, .. } | Self::Refusal { value, .. } => value,
        }
    }

    fn stream_done(&self) -> bool {
        match self {
            Self::Text { stream_done, .. } | Self::Refusal { stream_done, .. } => *stream_done,
        }
    }

    fn declared(&self) -> bool {
        match self {
            Self::Text { declared, .. } | Self::Refusal { declared, .. } => *declared,
        }
    }

    fn part_done(&self) -> bool {
        match self {
            Self::Text { part_done, .. } | Self::Refusal { part_done, .. } => *part_done,
        }
    }

    fn matches_type(&self, part_type: &str) -> bool {
        matches!(
            (self, part_type),
            (Self::Text { .. }, "output_text") | (Self::Refusal { .. }, "refusal")
        )
    }

    fn mark_part_done(&mut self) {
        match self {
            Self::Text { part_done, .. } | Self::Refusal { part_done, .. } => *part_done = true,
        }
    }
}

impl TextPart {
    fn implicit() -> Self {
        Self {
            value: String::new(),
            stream_done: false,
            declared: false,
            part_done: false,
        }
    }

    fn declared(value: String) -> Self {
        Self {
            value,
            stream_done: false,
            declared: true,
            part_done: false,
        }
    }

    fn is_done(&self) -> bool {
        self.stream_done && (!self.declared || self.part_done)
    }

    fn finish_wrapper(&mut self, final_text: &str) -> Result<(), ConnectorError> {
        if !self.stream_done || !self.declared || self.part_done || self.value != final_text {
            return Err(protocol_failure(
                "final reasoning part disagrees with its accumulated stream",
            ));
        }
        self.part_done = true;
        Ok(())
    }
}

fn value_at<'a>(
    value: &'a Value,
    path: &[&str],
    label: &'static str,
) -> Result<&'a Value, ConnectorError> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .ok_or_else(|| protocol_failure(format!("Responses event is missing {label}")))?;
    }
    Ok(current)
}

fn string_at<'a>(
    value: &'a Value,
    path: &[&str],
    label: &'static str,
) -> Result<&'a str, ConnectorError> {
    value_at(value, path, label)?
        .as_str()
        .ok_or_else(|| protocol_failure(format!("Responses {label} is not a string")))
}

fn optional_string_at(value: &Value, path: &[&str]) -> Result<Option<String>, ConnectorError> {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return Ok(None);
        };
        if next.is_null() {
            return Ok(None);
        }
        current = next;
    }
    current
        .as_str()
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| protocol_failure("optional Responses field is not a string"))
}

fn usize_at(value: &Value, path: &[&str], label: &'static str) -> Result<usize, ConnectorError> {
    let value = value_at(value, path, label)?
        .as_u64()
        .ok_or_else(|| protocol_failure(format!("Responses {label} is not an unsigned integer")))?;
    usize::try_from(value)
        .map_err(|_| protocol_failure(format!("Responses {label} is outside the host range")))
}

fn usage_at(event: &Value) -> Result<ResponsesUsage, ConnectorError> {
    let Some(usage) = event
        .get("response")
        .and_then(|response| response.get("usage"))
    else {
        return Ok(ResponsesUsage::default());
    };
    Ok(ResponsesUsage {
        input_tokens: optional_u64_at(usage, &["input_tokens"])?,
        output_tokens: optional_u64_at(usage, &["output_tokens"])?,
        total_tokens: optional_u64_at(usage, &["total_tokens"])?,
        reasoning_tokens: optional_u64_at(usage, &["output_tokens_details", "reasoning_tokens"])?,
    })
}

fn optional_u64_at(value: &Value, path: &[&str]) -> Result<Option<u64>, ConnectorError> {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return Ok(None);
        };
        if next.is_null() {
            return Ok(None);
        }
        current = next;
    }
    current
        .as_u64()
        .map(Some)
        .ok_or_else(|| protocol_failure("Responses usage field is not an unsigned integer"))
}

fn protocol_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Protocol, message)
}

fn limit_failure(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new(ConnectorFailureKind::Limit, message)
}
