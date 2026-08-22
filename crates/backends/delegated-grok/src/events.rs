use serde_json::Value;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityUpdate, BackendEvent,
    BackendFailure, BackendFailureKind, BackendOutcomeEvidence, BackendPoll, Failure, TurnOutcome,
};

use super::{
    ApprovalBinding, Backend, MessageBinding, MessageChannel, MessageKey, ToolBinding,
    client::ClientPoll,
    protocol::{self, Incoming},
    transport::JsonPeer,
};

const MAX_ACP_IDENTIFIER_BYTES: usize = 4096;

impl<P: JsonPeer> Backend<P> {
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
                Incoming::Notification { method, params } => {
                    self.map_notification(&method, params)?
                },
                Incoming::ServerRequest { id, method, params } => {
                    self.map_server_request(id, &method, params)?
                },
                Incoming::Response { id, result } => self.prompt_completed(id, &result)?,
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
            "user_message_chunk"
            | "plan"
            | "available_commands_update"
            | "current_mode_update"
            | "config_option_update"
            | "session_info_update"
            | "usage_update" => Ok(None),
            _ => Ok(None),
        }
    }

    fn message_chunk(
        &mut self,
        update: &Value,
        channel: MessageChannel,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let turn = self.active_turn()?;
        let content = update
            .get("content")
            .ok_or_else(|| protocol::protocol_failure("Grok ACP message chunk has no content"))?;
        if content.get("type").and_then(Value::as_str) != Some("text") {
            return Ok(None);
        }
        let text = protocol::string_at(content, &["text"])?;
        let key = MessageKey {
            channel,
            message_id: optional_identifier(update, "messageId")?,
        };
        let existing = self.messages.get(&key).map(|binding| binding.activity);
        let activity = match existing {
            Some(activity) => activity,
            None => {
                self.ensure_activity_capacity()?;
                let activity = self.next_activity(turn)?;
                self.messages.insert(key, MessageBinding { activity });
                self.pending_events
                    .push_back(BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextDelta(text.to_owned()),
                    });
                return Ok(Some(BackendEvent::ActivityStarted {
                    activity,
                    kind: match channel {
                        MessageChannel::Agent => ActivityKind::AgentMessage,
                        MessageChannel::Thought => ActivityKind::ModelWork,
                    },
                }));
            },
        };
        Ok(Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta(text.to_owned()),
        }))
    }

    fn tool_call(&mut self, update: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        let turn = self.active_turn()?;
        let tool_id = identifier_at(update, "toolCallId")?.to_owned();
        if self.seen_tool_ids.contains(&tool_id) {
            return Err(protocol::protocol_failure(format!(
                "duplicate Grok ACP tool call `{tool_id}`"
            )));
        }
        if self.seen_tool_ids.len() >= Self::MAX_SESSION_TOOL_IDS {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP exceeded the per-Session ToolCallId limit of {}",
                Self::MAX_SESSION_TOOL_IDS
            )));
        }
        self.seen_tool_ids.insert(tool_id.clone());
        if let Err(failure) = self.ensure_activity_capacity() {
            self.seen_tool_ids.remove(&tool_id);
            return Err(failure);
        }
        let activity = self.next_activity(turn)?;
        self.tools.insert(
            tool_id,
            ToolBinding {
                activity,
                finished: false,
            },
        );
        if let Some(title) = update.get("title").and_then(Value::as_str) {
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(title.to_owned()),
                });
        }
        self.queue_tool_terminal(update, activity)?;
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: activity_kind(update.get("kind").and_then(Value::as_str)),
        }))
    }

    fn tool_call_update(&mut self, update: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        self.active_turn()?;
        let tool_id = identifier_at(update, "toolCallId")?;
        let binding = self.tools.get(tool_id).ok_or_else(|| {
            protocol::protocol_failure(format!(
                "Grok ACP update targets unknown tool call `{tool_id}`"
            ))
        })?;
        if binding.finished {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP update targets completed tool call `{tool_id}`"
            )));
        }
        let activity = binding.activity;
        let mut updates = Vec::new();
        if let Some(title) = update.get("title").and_then(Value::as_str) {
            updates.push(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(title.to_owned()),
            });
        }
        if let Some(snapshot) = tool_content_snapshot(update) {
            updates.push(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(snapshot),
            });
        }
        if let Some(terminal) = tool_terminal(update, activity)? {
            self.tools
                .get_mut(tool_id)
                .expect("validated tool binding remains present")
                .finished = true;
            updates.push(terminal);
        }
        let mut events = updates.into_iter();
        let first = events.next();
        self.pending_events.extend(events);
        Ok(first)
    }

    fn queue_tool_terminal(
        &mut self,
        update: &Value,
        activity: yo_core::ActivityRef,
    ) -> Result<(), BackendFailure> {
        if let Some(terminal) = tool_terminal(update, activity)? {
            let tool_id = identifier_at(update, "toolCallId")?;
            self.tools
                .get_mut(tool_id)
                .expect("new tool binding remains present")
                .finished = true;
            self.pending_events.push_back(terminal);
        }
        Ok(())
    }

    fn map_server_request(
        &mut self,
        wire_id: Value,
        method: &str,
        params: Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        if method != "session/request_permission" {
            self.client
                .reject(wire_id, -32601, "client request is unsupported by yo")?;
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                format!("unsupported Grok ACP client request `{method}`"),
            ));
        }
        self.validate_session(&params)?;
        let turn = self.active_turn()?;
        if self
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.interrupt_requested)
        {
            self.client.respond(
                wire_id,
                serde_json::json!({ "outcome": { "outcome": "cancelled" } }),
            )?;
            return Ok(None);
        }
        let options = params
            .get("options")
            .and_then(Value::as_array)
            .ok_or_else(|| protocol::protocol_failure("Grok permission request has no options"))?;
        let allow_option = permission_option(options, "allow_once").ok_or_else(|| {
            protocol::protocol_failure("Grok permission request has no allow_once option")
        })?;
        let reject_option = permission_option(options, "reject_once").ok_or_else(|| {
            protocol::protocol_failure("Grok permission request has no reject_once option")
        })?;
        self.ensure_activity_capacity()?;
        let activity = self.next_activity(turn)?;
        let request_id = self.next_request()?;
        let request = ActivityRequestRef::new(activity, request_id);
        let wire_key = wire_key(&wire_id)?;
        if self.wire_approvals.contains_key(&wire_key) {
            return Err(protocol::protocol_failure(
                "duplicate Grok ACP permission request id",
            ));
        }
        self.approvals.insert(
            request,
            ApprovalBinding {
                wire_id,
                activity,
                allow_option,
                reject_option,
            },
        );
        self.wire_approvals.insert(wire_key, request);
        if let Some(title) = params.pointer("/toolCall/title").and_then(Value::as_str) {
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(title.to_owned()),
                });
        }
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }))
    }

    fn prompt_completed(
        &mut self,
        response_id: u64,
        result: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let prompt = self.prompt.take().ok_or_else(|| {
            protocol::protocol_failure("Grok ACP prompt response has no active Turn")
        })?;
        if prompt.request_id != response_id {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP completed prompt {response_id} instead of {}",
                prompt.request_id
            )));
        }
        if !self.approvals.is_empty() {
            return Err(protocol::protocol_failure(
                "Grok ACP completed a prompt with an unresolved permission request",
            ));
        }
        let stop_reason = protocol::string_at(result, &["stopReason"])?;
        if prompt.interrupt_requested && stop_reason != "cancelled" {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP returned `{stop_reason}` after session/cancel"
            )));
        }
        let outcome = match stop_reason {
            "end_turn" => TurnOutcome::Completed,
            "cancelled" => TurnOutcome::Interrupted,
            "max_tokens" => TurnOutcome::Failed(Failure::new("Grok reached its token limit")),
            "max_turn_requests" => {
                TurnOutcome::Failed(Failure::new("Grok reached its agent request limit"))
            },
            "refusal" => TurnOutcome::Failed(Failure::new("Grok refused the request")),
            other => {
                return Err(protocol::protocol_failure(format!(
                    "Grok ACP returned unsupported stop reason `{other}`"
                )));
            },
        };
        let activity_outcome = match &outcome {
            TurnOutcome::Completed => ActivityOutcome::Completed,
            TurnOutcome::Interrupted => ActivityOutcome::Interrupted,
            TurnOutcome::Failed(failure) => ActivityOutcome::Failed(failure.clone()),
        };
        let mut activities = self
            .messages
            .drain()
            .map(|(_, binding)| binding.activity)
            .chain(
                self.tools
                    .drain()
                    .filter_map(|(_, binding)| (!binding.finished).then_some(binding.activity)),
            )
            .chain(self.approvals.drain().map(|(_, binding)| binding.activity))
            .collect::<Vec<_>>();
        self.wire_approvals.clear();
        activities.sort_unstable();
        activities.dedup();
        self.pending_events
            .extend(
                activities
                    .into_iter()
                    .map(|activity| BackendEvent::ActivityFinished {
                        activity,
                        outcome: activity_outcome.clone(),
                    }),
            );
        let turn_finished = if outcome == TurnOutcome::Completed && self.load_session {
            BackendEvent::ResumableTurnFinished {
                turn: prompt.turn,
                evidence: BackendOutcomeEvidence::without_identity(),
            }
        } else {
            BackendEvent::TurnFinished {
                turn: prompt.turn,
                outcome,
            }
        };
        self.pending_events.push_back(turn_finished);
        Ok(self.pending_events.pop_front())
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

fn activity_kind(kind: Option<&str>) -> ActivityKind {
    match kind {
        Some("edit" | "delete" | "move") => ActivityKind::FileChange,
        Some("think") => ActivityKind::ModelWork,
        _ => ActivityKind::ToolCall,
    }
}

fn tool_terminal(
    update: &Value,
    activity: yo_core::ActivityRef,
) -> Result<Option<BackendEvent>, BackendFailure> {
    let Some(status) = update.get("status").and_then(Value::as_str) else {
        return Ok(None);
    };
    let outcome = match status {
        "pending" | "in_progress" => return Ok(None),
        "completed" => ActivityOutcome::Completed,
        "failed" => ActivityOutcome::Failed(Failure::new("Grok tool call failed")),
        other => {
            return Err(protocol::protocol_failure(format!(
                "Grok tool call has invalid status `{other}`"
            )));
        },
    };
    Ok(Some(BackendEvent::ActivityFinished { activity, outcome }))
}

fn tool_content_snapshot(update: &Value) -> Option<String> {
    update.get("content")?.as_array()?.iter().find_map(|item| {
        (item.get("type")?.as_str()? == "content")
            .then(|| item.pointer("/content/text")?.as_str().map(str::to_owned))?
    })
}

fn permission_option(options: &[Value], kind: &str) -> Option<String> {
    options.iter().find_map(|option| {
        (option.get("kind").and_then(Value::as_str) == Some(kind)).then(|| {
            option
                .get("optionId")?
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= MAX_ACP_IDENTIFIER_BYTES)
                .map(str::to_owned)
        })?
    })
}

fn identifier_at<'a>(value: &'a Value, field: &str) -> Result<&'a str, BackendFailure> {
    let identifier = protocol::string_at(value, &[field])?;
    if identifier.is_empty() || identifier.len() > MAX_ACP_IDENTIFIER_BYTES {
        return Err(protocol::protocol_failure(format!(
            "Grok ACP field `{field}` is not a bounded identifier"
        )));
    }
    Ok(identifier)
}

fn optional_identifier(value: &Value, field: &str) -> Result<Option<String>, BackendFailure> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => identifier_at(value, field).map(|identifier| Some(identifier.to_owned())),
    }
}

fn wire_key(value: &Value) -> Result<String, BackendFailure> {
    serde_json::to_string(value)
        .map_err(|error| protocol::protocol_failure(format!("invalid Grok request id: {error}")))
}
