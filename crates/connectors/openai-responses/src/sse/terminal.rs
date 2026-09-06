use super::{
    ConnectorError, ModelConnectorEvent, ModelConnectorTerminal, ResponsesSseDecoder, Value,
    protocol_failure, string_at, usage_at,
};

impl ResponsesSseDecoder {
    pub(super) fn terminal(
        &mut self,
        event: &Value,
        status: ModelConnectorTerminal,
    ) -> Result<Vec<ModelConnectorEvent>, ConnectorError> {
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
            ModelConnectorTerminal::Completed => "completed",
            ModelConnectorTerminal::Incomplete { .. } => "incomplete",
            ModelConnectorTerminal::Failed { .. } => "failed",
        };
        if response_status != expected_status {
            return Err(protocol_failure(
                "terminal event type disagrees with response status",
            ));
        }
        let usage = usage_at(event)?;
        self.terminated = true;
        self.pending_terminal = Some(ModelConnectorEvent::Terminal {
            response_id,
            status,
            usage,
        });
        Ok(Vec::new())
    }
}
