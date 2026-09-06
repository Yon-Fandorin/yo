use super::{
    ConnectorError, HashMap, MessageContent, ModelConnectorEvent, OutputItem, OutputItemKind,
    ReasoningChannel, ReasoningPart, ResponsesSseDecoder, TextPart, Value, limit_failure,
    protocol_failure, string_at, usize_at, value_at,
};

impl ResponsesSseDecoder {
    pub(super) fn response_created(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
        let response_id = string_at(event, &["response", "id"], "response id")?.to_owned();
        if self.response_id.replace(response_id.clone()).is_some() {
            return Err(protocol_failure("duplicate response.created event"));
        }
        Ok(vec![ModelConnectorEvent::ResponseCreated { response_id }])
    }

    pub(super) fn output_item_added(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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
                let emitted = vec![ModelConnectorEvent::FunctionCallStarted {
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

    pub(super) fn output_item_done(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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
                vec![ModelConnectorEvent::MessageDone {
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
                    vec![ModelConnectorEvent::FunctionCallDone {
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

    pub(super) fn content_part_added(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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

    pub(super) fn content_part_done(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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

    pub(super) fn reasoning_summary_part_added(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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

    pub(super) fn reasoning_summary_part_done(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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
}
