use serde_json::Value;

use super::{
    ApprovalBinding, Backend, ItemBinding,
    client::ClientPoll,
    protocol::{self, Incoming},
    transport::JsonPeer,
};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityUpdate, BackendEvent,
    BackendFailure, BackendFailureKind, BackendPoll, Failure, TurnOutcome,
};

impl<P: JsonPeer> Backend<P> {
    pub(super) fn poll_client_message(&mut self) -> Result<BackendPoll, BackendFailure> {
        loop {
            let incoming = match self.client.poll()? {
                ClientPoll::Pending => return Ok(BackendPoll::Pending),
                ClientPoll::Closed => return Ok(BackendPoll::Closed),
                ClientPoll::Message(incoming) => incoming,
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
            "item/agentMessage/delta" | "item/commandExecution/outputDelta" | "item/plan/delta" => {
                self.item_delta(&params)
            },
            "turn/completed" => self.turn_completed(&params),
            "serverRequest/resolved" => self.server_request_resolved(&params),
            "error" => self.record_turn_error(&params),
            "warning" | "configWarning" | "turn/diff/updated" | "turn/plan/updated" => Ok(None),
            _ => Ok(None),
        }
    }

    fn item_started(&mut self, params: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let turn = self
            .wire_turns
            .get(wire_turn)
            .map(|binding| binding.turn)
            .ok_or_else(|| {
                protocol::protocol_failure(format!("unknown Codex Turn `{wire_turn}`"))
            })?;
        let item_id = protocol::string_at(params, &["item", "id"])?.to_owned();
        let item_type = protocol::string_at(params, &["item", "type"])?;
        let Some(kind) = activity_kind(item_type) else {
            return Ok(None);
        };
        let activity = self.next_activity(turn)?;
        if self
            .items
            .insert(item_id.clone(), ItemBinding { activity })
            .is_some()
        {
            return Err(protocol::protocol_failure(format!(
                "duplicate Codex item `{item_id}`"
            )));
        }
        Ok(Some(BackendEvent::ActivityStarted { activity, kind }))
    }

    fn item_completed(&mut self, params: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let wire_binding = self.wire_turns.get(wire_turn).copied().ok_or_else(|| {
            protocol::protocol_failure(format!("unknown Codex Turn `{wire_turn}`"))
        })?;
        let turn = wire_binding.turn;
        let item_id = protocol::string_at(params, &["item", "id"])?;
        let item_type = protocol::string_at(params, &["item", "type"])?;
        let supported = activity_kind(item_type).is_some();
        let binding = self.items.get(item_id);
        if binding.is_some_and(|binding| binding.activity.turn() != turn) {
            if supported && wire_binding.interrupted {
                return Ok(None);
            }
            return Err(protocol::protocol_failure(format!(
                "completed Codex item `{item_id}` changed Turn"
            )));
        }
        let Some(binding) = self.items.remove(item_id) else {
            if supported && wire_binding.interrupted {
                return Ok(None);
            }
            return if activity_kind(item_type).is_some() {
                Err(protocol::protocol_failure(format!(
                    "completed Codex item `{item_id}` was not started"
                )))
            } else {
                Ok(None)
            };
        };
        let status = params
            .pointer("/item/status")
            .and_then(Value::as_str)
            .unwrap_or("completed");
        let outcome = match status {
            "completed" => ActivityOutcome::Completed,
            "declined" => ActivityOutcome::Interrupted,
            "failed" => {
                ActivityOutcome::Failed(Failure::new(format!("Codex item `{item_id}` failed")))
            },
            other => {
                return Err(protocol::protocol_failure(format!(
                    "Codex item `{item_id}` completed with invalid status `{other}`"
                )));
            },
        };
        let finished = BackendEvent::ActivityFinished {
            activity: binding.activity,
            outcome,
        };
        if let Some(snapshot) = final_text_snapshot(params) {
            self.pending_events.push_back(finished);
            return Ok(Some(BackendEvent::ActivityUpdated {
                activity: binding.activity,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }));
        }
        Ok(Some(finished))
    }

    fn item_delta(&self, params: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let turn = self
            .wire_turns
            .get(wire_turn)
            .map(|binding| binding.turn)
            .ok_or_else(|| {
                protocol::protocol_failure(format!("unknown Codex Turn `{wire_turn}`"))
            })?;
        let item_id = protocol::string_at(params, &["itemId"])?;
        let Some(binding) = self.items.get(item_id) else {
            return Err(protocol::protocol_failure(format!(
                "delta targets unknown Codex item `{item_id}`"
            )));
        };
        if binding.activity.turn() != turn {
            return Err(protocol::protocol_failure(format!(
                "Codex item delta `{item_id}` changed Turn"
            )));
        }
        let delta = protocol::string_at(params, &["delta"])?;
        Ok(Some(BackendEvent::ActivityUpdated {
            activity: binding.activity,
            update: ActivityUpdate::TextDelta(delta.to_owned()),
        }))
    }

    fn turn_completed(&mut self, params: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turn", "id"])?;
        let turn = self
            .wire_turns
            .get(wire_turn)
            .map(|binding| binding.turn)
            .ok_or_else(|| {
                protocol::protocol_failure(format!("unknown completed Codex Turn `{wire_turn}`"))
            })?;
        let status = protocol::string_at(params, &["turn", "status"])?;
        let outcome = match status {
            "completed" => TurnOutcome::Completed,
            "interrupted" => TurnOutcome::Interrupted,
            "failed" => {
                let message = self.turn_errors.remove(wire_turn).unwrap_or_else(|| {
                    params
                        .pointer("/turn/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex Turn failed")
                        .to_owned()
                });
                TurnOutcome::Failed(Failure::new(message))
            },
            other => {
                return Err(protocol::protocol_failure(format!(
                    "Codex Turn completed with invalid status `{other}`"
                )));
            },
        };
        let turn_finished = BackendEvent::TurnFinished {
            turn,
            outcome: outcome.clone(),
        };
        if outcome != TurnOutcome::Interrupted {
            return Ok(Some(turn_finished));
        }
        self.wire_turns
            .get_mut(wire_turn)
            .expect("the completed Codex Turn binding was resolved above")
            .interrupted = true;

        let interrupted_items = self
            .items
            .iter()
            .filter(|(_, binding)| binding.activity.turn() == turn)
            .map(|(item_id, binding)| (item_id.clone(), binding.activity))
            .collect::<Vec<_>>();
        let interrupted_approvals = self
            .approvals
            .iter()
            .filter(|(_, binding)| binding.request_activity.turn() == turn)
            .map(|(request, binding)| (*request, binding.request_activity))
            .collect::<Vec<_>>();

        let mut interrupted_activities =
            Vec::with_capacity(interrupted_items.len() + interrupted_approvals.len());
        for (item_id, activity) in interrupted_items {
            self.items.remove(&item_id);
            interrupted_activities.push(activity);
        }
        for (request, activity) in interrupted_approvals {
            let approval = self
                .approvals
                .remove(&request)
                .expect("collected approval binding must still exist");
            let key = wire_key(&approval.wire_id)?;
            self.wire_approvals.remove(&key);
            interrupted_activities.push(activity);
        }
        interrupted_activities.sort_unstable();

        let mut terminal_events = interrupted_activities
            .into_iter()
            .map(|activity| BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Interrupted,
            })
            .chain(std::iter::once(turn_finished));
        let first = terminal_events
            .next()
            .expect("an interrupted Turn always has its own terminal event");
        self.pending_events.extend(terminal_events);
        Ok(Some(first))
    }

    fn record_turn_error(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let message = protocol::string_at(params, &["error", "message"])?.to_owned();
        let Some(wire_turn) = params.get("turnId").and_then(Value::as_str) else {
            return Err(BackendFailure::new(BackendFailureKind::Turn, message));
        };
        if !self.wire_turns.contains_key(wire_turn) {
            return Err(protocol::protocol_failure(format!(
                "error targets unknown Codex Turn `{wire_turn}`"
            )));
        }
        self.turn_errors.insert(wire_turn.to_owned(), message);
        Ok(None)
    }

    fn map_server_request(
        &mut self,
        wire_id: Value,
        method: &str,
        params: Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        if !matches!(
            method,
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
        ) {
            self.client
                .reject(wire_id, -32601, "server request is unsupported by yo")?;
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!("unsupported Codex server request `{method}`"),
            ));
        }
        self.validate_thread(&params)?;
        let wire_turn = protocol::string_at(&params, &["turnId"])?;
        let turn = self
            .wire_turns
            .get(wire_turn)
            .map(|binding| binding.turn)
            .ok_or_else(|| {
                protocol::protocol_failure(format!("approval targets unknown Turn `{wire_turn}`"))
            })?;
        let activity = self.next_activity(turn)?;
        let request_id = self.next_request()?;
        let request = ActivityRequestRef::new(activity, request_id);
        let wire_key = wire_key(&wire_id)?;
        self.approvals.insert(
            request,
            ApprovalBinding {
                wire_id,
                request_activity: activity,
            },
        );
        if self.wire_approvals.insert(wire_key, request).is_some() {
            return Err(protocol::protocol_failure(
                "duplicate Codex approval request id",
            ));
        }
        if let Some(summary) = approval_summary(method, &params) {
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(summary),
                });
        }
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }))
    }

    fn server_request_resolved(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_id = params.get("requestId").ok_or_else(|| {
            protocol::protocol_failure("resolved server request has no requestId")
        })?;
        let key = wire_key(wire_id)?;
        let Some(request) = self.wire_approvals.remove(&key) else {
            return Ok(None);
        };
        let approval = self.approvals.remove(&request).ok_or_else(|| {
            protocol::protocol_failure("resolved approval lost its request binding")
        })?;
        Ok(Some(BackendEvent::ActivityFinished {
            activity: approval.request_activity,
            outcome: ActivityOutcome::Completed,
        }))
    }

    fn validate_thread(&self, params: &Value) -> Result<(), BackendFailure> {
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

fn activity_kind(item_type: &str) -> Option<ActivityKind> {
    match item_type {
        "agentMessage" => Some(ActivityKind::AgentMessage),
        "reasoning" | "plan" => Some(ActivityKind::ModelWork),
        "commandExecution" | "mcpToolCall" | "dynamicToolCall" => Some(ActivityKind::ToolCall),
        "fileChange" => Some(ActivityKind::FileChange),
        _ => None,
    }
}

fn wire_key(value: &Value) -> Result<String, BackendFailure> {
    serde_json::to_string(value)
        .map_err(|error| protocol::protocol_failure(format!("invalid Codex request id: {error}")))
}

fn final_text_snapshot(params: &Value) -> Option<String> {
    let item = params.get("item")?;
    match item.get("type")?.as_str()? {
        "agentMessage" | "plan" => item.get("text")?.as_str().map(str::to_owned),
        "commandExecution" => command_snapshot(item),
        "fileChange" => file_change_snapshot(item),
        _ => None,
    }
}

fn command_snapshot(item: &Value) -> Option<String> {
    let command = item.get("command").and_then(Value::as_str);
    let output = item.get("aggregatedOutput").and_then(Value::as_str);
    match (command, output.filter(|output| !output.is_empty())) {
        (Some(command), Some(output)) => Some(format!("$ {command}\n{output}")),
        (Some(command), None) => Some(format!("$ {command}")),
        (None, Some(output)) => Some(output.to_owned()),
        (None, None) => None,
    }
}

fn file_change_snapshot(item: &Value) -> Option<String> {
    let changes = item.get("changes")?.as_array()?;
    let lines = changes
        .iter()
        .filter_map(|change| {
            let path = change.get("path")?.as_str()?;
            let kind = change
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("update");
            Some(format!("{kind}: {path}"))
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn approval_summary(method: &str, params: &Value) -> Option<String> {
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.is_empty());
    let subject = match method {
        "item/commandExecution/requestApproval" => params
            .get("command")
            .and_then(Value::as_str)
            .map(|command| format!("$ {command}")),
        "item/fileChange/requestApproval" => params
            .get("grantRoot")
            .and_then(Value::as_str)
            .map(|root| format!("write access: {root}")),
        _ => None,
    };
    match (subject, reason) {
        (Some(subject), Some(reason)) => Some(format!("{subject}\nReason: {reason}")),
        (Some(subject), None) => Some(subject),
        (None, Some(reason)) => Some(format!("Reason: {reason}")),
        (None, None) => None,
    }
}
