use super::{
    ConnectorError, MessageContent, ModelConnectorEvent, OutputItemKind, ReasoningChannel,
    ReasoningPart, ResponsesSseDecoder, TextPart, Value, protocol_failure, string_at, usize_at,
};

impl ResponsesSseDecoder {
    pub(super) fn output_text_delta(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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
        Ok(vec![ModelConnectorEvent::TextDelta {
            output_index,
            item_id,
            content_index,
            delta,
        }])
    }

    pub(super) fn output_text_done(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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

    pub(super) fn refusal_delta(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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
        Ok(vec![ModelConnectorEvent::RefusalDelta {
            output_index,
            item_id,
            content_index,
            delta,
        }])
    }

    pub(super) fn refusal_done(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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

    pub(super) fn reasoning_delta(
        &mut self,
        event: &Value,
        channel: ReasoningChannel,
        index_field: &'static str,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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
        Ok(vec![ModelConnectorEvent::ReasoningDelta {
            output_index,
            item_id,
            channel,
            part_index,
            delta,
        }])
    }

    pub(super) fn reasoning_done(
        &mut self,
        event: &Value,
        channel: ReasoningChannel,
        index_field: &'static str,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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

    pub(super) fn function_arguments_delta(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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
        Ok(vec![ModelConnectorEvent::FunctionArgumentsDelta {
            output_index,
            item_id,
            delta,
        }])
    }

    pub(super) fn function_arguments_done(
        &mut self,
        event: &Value,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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
        Ok(vec![ModelConnectorEvent::FunctionCallDone {
            output_index,
            item_id,
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        }])
    }
}
