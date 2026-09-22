mod completion;
mod message;
mod permission;
mod question;
mod tool;
mod usage;

use serde_json::Value;
use yo_core::{BackendEvent, BackendFailure, BackendPoll};

use super::state::{Backend, MessageChannel};
use crate::{
    client::ClientPoll,
    protocol::{self, Incoming},
    transport::JsonPeer,
};

impl<P: JsonPeer> Backend<P> {
    pub(super) fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(BackendPoll::Event(event));
        }
        if let Some(event) = self.poll_secret_probe()? {
            return Ok(BackendPoll::Event(event));
        }
        self.poll_client_message()
    }

    pub(super) fn poll_client_message(&mut self) -> Result<BackendPoll, BackendFailure> {
        loop {
            let incoming = match self
                .client
                .poll(self.prompt.as_ref().map(|prompt| prompt.request_id))?
            {
                ClientPoll::Pending => return Ok(BackendPoll::Pending),
                ClientPoll::Closed => return Ok(BackendPoll::Closed),
                ClientPoll::Message(incoming) => incoming,
            };
            let event = match incoming {
                Incoming::MaintenanceAck => None,
                Incoming::Notification { method, params } => {
                    self.map_notification(&method, params)?
                },
                Incoming::ServerRequest { id, method, params } => {
                    self.map_server_request(id, &method, params)?
                },
                Incoming::Response { id, result } => {
                    completion::prompt_completed(self, id, &result, usage::prompt_usage_receipt)?
                },
                Incoming::ResponseError { .. } => {
                    return Err(protocol::protocol_failure(
                        "Grok ACP error response reached the event mapper",
                    ));
                },
            };
            if let Some(event) = event {
                return Ok(BackendPoll::Event(event));
            }
            if let Some(event) = self.pending_events.pop_front() {
                return Ok(BackendPoll::Event(event));
            }
        }
    }

    fn map_notification(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        match method {
            "session/update" => self.session_update(&params),
            _ => Ok(None),
        }
    }

    fn session_update(&mut self, params: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_session(params)?;
        let update = params.get("update").ok_or_else(|| {
            protocol::protocol_failure("Grok ACP session/update has no update value")
        })?;
        let update_kind = protocol::string_at(update, &["sessionUpdate"])?;
        match update_kind {
            "agent_message_chunk" => self.message_chunk(update, MessageChannel::Agent),
            "agent_thought_chunk" => self.message_chunk(update, MessageChannel::Thought),
            "tool_call" => self.tool_call(update),
            "tool_call_update" => self.tool_call_update(update),
            "plan" => self.plan_update(update),
            "user_message_chunk"
            | "available_commands_update"
            | "current_mode_update"
            | "config_option_update"
            | "session_info_update"
            // ACP usage_update reports cumulative Session/context state, while
            // Session Usage accepts completed per-prompt receipts only.
            | "usage_update" => Ok(None),
            _ => Ok(None),
        }
    }

    fn map_server_request(
        &mut self,
        wire_id: Value,
        method: &str,
        params: Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        match method {
            "session/request_permission" => {
                permission::map_server_request(self, wire_id, method, params)
            },
            "_x.ai/ask_user_question" => question::map_server_request(self, wire_id, params),
            _ => {
                self.client
                    .reject(wire_id, -32601, "client request is unsupported by yo")?;
                Err(BackendFailure::new(
                    yo_core::BackendFailureKind::Unsupported,
                    format!("unsupported Grok ACP client request `{method}`"),
                ))
            },
        }
    }

    fn validate_session(&self, params: &Value) -> Result<(), BackendFailure> {
        let observed = protocol::string_at(params, &["sessionId"])?;
        let expected = self
            .session
            .as_ref()
            .map(|session| session.grok.as_str())
            .ok_or_else(|| protocol::protocol_failure("Grok ACP Session binding was not found"))?;
        if observed != expected {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP update targets Session `{observed}` instead of `{expected}`"
            )));
        }
        Ok(())
    }
}
