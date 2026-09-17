//! Codex 알림 파사드와 디스패치.

mod interaction;
mod items;
mod lifecycle;
mod turn;
mod usage;

use serde_json::Value;
use yo_backend::transport::JsonMessagePeer;
use yo_core::{BackendEvent, BackendFailure, BackendPoll};

use super::super::state::Backend;
use crate::{
    client::ClientPoll,
    protocol::{self, Incoming},
};

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn poll_client_message(&mut self) -> Result<BackendPoll, BackendFailure> {
        loop {
            let incoming = match self.client.poll() {
                Ok(ClientPoll::Pending) => return Ok(BackendPoll::Pending),
                Ok(ClientPoll::Closed) => return self.close_connection(Ok(())),
                Err(failure) => return self.close_connection(Err(failure)),
                Ok(ClientPoll::Message(incoming)) => incoming,
            };
            let event = match incoming {
                Incoming::Notification { method, params } => {
                    self.map_notification(&method, params)?
                },
                Incoming::ServerRequest { id, method, params } => {
                    self.map_server_request(id, &method, params)?
                },
                Incoming::Response { .. } | Incoming::ResponseError { .. } => {
                    return Err(protocol::protocol_failure(
                        "Codex response reached the event stream",
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
            "thread/started" | "turn/started" | "thread/status/changed" => Ok(None),
            "item/started" => self.item_started(&params),
            "item/completed" => self.item_completed(&params),
            "item/agentMessage/delta" | "item/commandExecution/outputDelta" => {
                self.item_delta(&params)
            },
            "item/commandExecution/terminalInteraction" => self.terminal_interaction(&params),
            "item/plan/delta" => self.proposed_plan_delta(&params),
            "item/reasoning/summaryTextDelta" => self.reasoning_summary_delta(&params),
            "thread/tokenUsage/updated" => self.token_usage_updated(&params),
            "turn/plan/updated" => self.plan_updated(&params),
            "model/rerouted" => self.model_rerouted(&params),
            "turn/completed" => self.turn_completed(&params),
            "serverRequest/resolved" => self.server_request_resolved(&params),
            "error" => self.record_turn_error(&params),
            "turn/diff/updated" => self.turn_diff_updated(&params),
            "warning" | "configWarning" => Ok(None),
            _ => Ok(None),
        }
    }

    pub(super) fn validate_thread(&self, params: &Value) -> Result<(), BackendFailure> {
        let wire_thread = protocol::string_at(params, &["threadId"])?;
        let expected = self
            .session
            .as_ref()
            .map(|binding| binding.codex.as_str())
            .ok_or_else(|| protocol::protocol_failure("Codex Session binding was not found"))?;
        if wire_thread != expected {
            return Err(protocol::protocol_failure(format!(
                "Codex event targets Thread `{wire_thread}` instead of `{expected}`"
            )));
        }
        Ok(())
    }
}
