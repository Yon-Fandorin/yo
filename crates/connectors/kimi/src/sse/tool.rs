use super::{
    ChatCompletionsSseDecoder, ConnectorError, KimiReplayToolCallSize, ToolCall, Value,
    encoded_json_string_payload_bytes, json, limit_failure, optional_string, protocol_failure,
    string_at, unsigned_at, wire::valid_kimi_function_name,
};

impl ChatCompletionsSseDecoder {
    pub(super) fn apply_tool_fragment(&mut self, fragment: &Value) -> Result<(), ConnectorError> {
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
}
