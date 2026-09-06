use super::{
    ChatCompletionsSseDecoder, ConnectorError, KimiAssistantMessage, KimiAssistantToolCall,
    ModelConnectorEvent, ModelConnectorUsage, Value, protocol_failure,
};

impl ChatCompletionsSseDecoder {
    pub(super) fn kimi_private_replay(&self) -> bool {
        self.private_replay
    }

    pub(super) fn validate_kimi_chunk_identity(&self, chunk: &Value) -> Result<(), ConnectorError> {
        if chunk.get("object").and_then(Value::as_str) != Some("chat.completion.chunk")
            || chunk.get("model").and_then(Value::as_str) != Some(self.model.as_str())
        {
            return Err(protocol_failure(
                "Kimi stream object or model does not match the request",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_kimi_delta_shape(
        &self,
        delta: &serde_json::Map<String, Value>,
    ) -> Result<(), ConnectorError> {
        const ALLOWED: [&str; 4] = ["role", "content", "reasoning_content", "tool_calls"];
        if delta.keys().any(|field| !ALLOWED.contains(&field.as_str())) {
            return Err(protocol_failure("Kimi delta contains an undeclared field"));
        }
        Ok(())
    }

    pub(super) fn validate_role(&mut self, role: Option<&str>) -> Result<(), ConnectorError> {
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

    pub(super) fn emit_kimi_private(
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

    pub(super) fn accept_usage(
        &mut self,
        usage: ModelConnectorUsage,
    ) -> Result<(), ConnectorError> {
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
}
