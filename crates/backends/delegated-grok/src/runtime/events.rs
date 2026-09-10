use std::time::Duration;

use serde_json::{Value, json};
use similar::TextDiff;
use yo_core::{
    ActivityApproval, ActivityKind, ActivityOutcome, ActivityPlan, ActivityReasoning,
    ActivityRequestRef, ActivityUpdate, ApprovalChoice, BackendEvent, BackendFailure,
    BackendFailureKind, BackendOutcomeEvidence, BackendPoll, Failure, MessageContent, PlanStep,
    PlanStepStatus, ToolOutput, TurnOutcome, TurnRef,
};

use super::{
    ApprovalBinding, Backend, MessageBinding, MessageChannel, MessageKey, ToolBinding, ToolIdentity,
};
use crate::{
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

    fn plan_update(&mut self, update: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        let turn = self.active_turn()?;
        let entries = update
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| protocol::protocol_failure("Grok ACP plan has no entries array"))?;
        let mut steps = Vec::new();
        let mut bytes = 0_usize;
        for entry in entries {
            let content = protocol::string_at(entry, &["content"])?;
            let priority = protocol::string_at(entry, &["priority"])?;
            if !matches!(priority, "high" | "medium" | "low") {
                return Err(protocol::protocol_failure(
                    "Grok ACP plan priority is unsupported",
                ));
            }
            let status = match protocol::string_at(entry, &["status"])? {
                "pending" => PlanStepStatus::Pending,
                "in_progress" => PlanStepStatus::InProgress,
                "completed" => PlanStepStatus::Completed,
                _ => {
                    return Err(protocol::protocol_failure(
                        "Grok ACP plan status is unsupported",
                    ));
                },
            };
            bytes = bytes
                .checked_add(content.len())
                .and_then(|size| size.checked_add(priority.len() + 3))
                .filter(|size| *size <= ToolOutput::MAX_SNAPSHOT_BYTES)
                .ok_or_else(|| {
                    protocol::protocol_failure("Grok ACP plan exceeds presentation limit")
                })?;
            steps.push(PlanStep {
                text: format!("[{priority}] {content}"),
                status,
            });
        }
        let text = ActivityPlan {
            explanation: None,
            steps,
        }
        .to_snapshot()
        .ok_or_else(|| protocol::protocol_failure("Grok ACP plan exceeds presentation limit"))?;
        let key = MessageKey {
            channel: MessageChannel::Plan,
            message_id: None,
        };
        if let Some(binding) = self.messages.get(&key) {
            return Ok(Some(BackendEvent::ActivityUpdated {
                activity: binding.activity,
                update: ActivityUpdate::TextSnapshot(text),
            }));
        }
        self.ensure_activity_capacity()?;
        let activity = self.next_activity(turn)?;
        self.messages.insert(
            key,
            MessageBinding {
                activity,
                reasoning: None,
            },
        );
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            });
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }))
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
        let key = MessageKey {
            channel,
            message_id: optional_identifier(update, "messageId")?,
        };
        if content.get("type").and_then(Value::as_str) != Some("text") {
            self.ensure_activity_capacity()?;
            let activity = self.next_activity(turn)?;
            // A content block splits this message's text stream so later text stays
            // after the image/resource, including streams with an explicit messageId.
            if let Some(binding) = self.messages.remove(&key) {
                self.pending_events
                    .push_back(BackendEvent::ActivityFinished {
                        activity: binding.activity,
                        outcome: ActivityOutcome::Completed,
                    });
            }
            let source = if channel == MessageChannel::Agent {
                MessageContent {
                    block: content.clone(),
                }
                .to_snapshot()
            } else {
                Some(reasoning_snapshot(content.clone())?)
            }
            .unwrap_or_else(|| format!("{content:#}"));
            self.pending_events.extend([
                BackendEvent::ActivityStarted {
                    activity,
                    kind: match channel {
                        MessageChannel::Agent => ActivityKind::AgentMessage,
                        MessageChannel::Thought | MessageChannel::Plan => ActivityKind::ModelWork,
                    },
                },
                BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(source),
                },
                BackendEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                },
            ]);
            return Ok(self.pending_events.pop_front());
        }
        let text = protocol::string_at(content, &["text"])?;
        if channel == MessageChannel::Thought {
            if let Some(binding) = self.messages.get_mut(&key) {
                let accumulated = binding
                    .reasoning
                    .as_mut()
                    .expect("thought stream owns text");
                if accumulated.len().saturating_add(text.len()) > ToolOutput::MAX_SNAPSHOT_BYTES {
                    return Err(protocol::protocol_failure(
                        "Grok reasoning exceeds presentation limit",
                    ));
                }
                accumulated.push_str(text);
                return Ok(Some(BackendEvent::ActivityUpdated {
                    activity: binding.activity,
                    update: ActivityUpdate::TextSnapshot(reasoning_snapshot(Value::String(
                        accumulated.clone(),
                    ))?),
                }));
            }
            let snapshot = reasoning_snapshot(Value::String(text.to_owned()))?;
            self.ensure_activity_capacity()?;
            let activity = self.next_activity(turn)?;
            self.messages.insert(
                key,
                MessageBinding {
                    activity,
                    reasoning: Some(text.to_owned()),
                },
            );
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(snapshot),
                });
            return Ok(Some(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            }));
        }
        let existing = self.messages.get(&key).map(|binding| binding.activity);
        let activity = match existing {
            Some(activity) => activity,
            None => {
                self.ensure_activity_capacity()?;
                let activity = self.next_activity(turn)?;
                self.messages.insert(
                    key,
                    MessageBinding {
                        activity,
                        reasoning: None,
                    },
                );
                self.pending_events
                    .push_back(BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextDelta(text.to_owned()),
                    });
                return Ok(Some(BackendEvent::ActivityStarted {
                    activity,
                    kind: match channel {
                        MessageChannel::Agent => ActivityKind::AgentMessage,
                        MessageChannel::Thought | MessageChannel::Plan => ActivityKind::ModelWork,
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
        self.finish_anonymous_messages();
        self.seen_tool_ids.insert(tool_id.clone());
        if let Err(failure) = self.ensure_activity_capacity() {
            self.seen_tool_ids.remove(&tool_id);
            return Err(failure);
        }
        let activity = self.next_activity(turn)?;
        let identity = tool_identity(update);
        let identity_snapshot = tool_identity_snapshot(&identity, &tool_id);
        self.tools.insert(
            tool_id.clone(),
            ToolBinding {
                activity,
                file_change: activity_kind(update.get("kind").and_then(Value::as_str))
                    == ActivityKind::FileChange,
                result_activity: None,
                output: json!({}),
                identity,
                finished: false,
            },
        );
        self.pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity,
                kind: activity_kind(update.get("kind").and_then(Value::as_str)),
            });
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(identity_snapshot),
            });
        let mut pending = self
            .approvals
            .iter()
            .filter(|(_, binding)| {
                binding.activity.turn() == turn
                    && binding.tool_call_id.as_deref() == Some(tool_id.as_str())
                    && binding.pending_display.is_some()
            })
            .map(|(request, _)| *request)
            .collect::<Vec<_>>();
        pending.sort_unstable();
        let mut progress = update.clone();
        for request in pending.iter().rev() {
            let display = self.approvals[request]
                .pending_display
                .as_ref()
                .expect("collected display");
            for field in ["rawInput", "content", "locations"] {
                if progress.get(field).is_none()
                    && let Some(value) = display.get(field)
                {
                    progress[field] = value.clone();
                }
            }
        }
        self.queue_tool_progress(&tool_id, &progress)?;
        for request in pending {
            self.approvals
                .get_mut(&request)
                .expect("collected approval")
                .pending_display = None;
        }
        if self
            .tools
            .get(&tool_id)
            .is_some_and(|binding| binding.file_change && !binding.finished)
        {
            let mut requests = self
                .approvals
                .iter()
                .filter(|(_, binding)| {
                    binding.activity.turn() == turn
                        && binding.tool_call_id.as_deref() == Some(tool_id.as_str())
                        && binding.profile.related_change.is_none()
                })
                .map(|(request, _)| *request)
                .collect::<Vec<_>>();
            requests.sort_unstable();
            for request in requests {
                let binding = self
                    .approvals
                    .get_mut(&request)
                    .expect("collected approval remains");
                binding.profile.related_change = Some(activity.activity_id().get().get());
                self.pending_events
                    .push_back(BackendEvent::ActivityUpdated {
                        activity: binding.activity,
                        update: ActivityUpdate::TextSnapshot(
                            binding
                                .profile
                                .to_snapshot()
                                .expect("approval reserved link capacity"),
                        ),
                    });
            }
        }
        Ok(self.pending_events.pop_front())
    }

    fn finish_anonymous_messages(&mut self) {
        let keys = self
            .messages
            .keys()
            .filter(|key| key.message_id.is_none() && key.channel != MessageChannel::Plan)
            .cloned()
            .collect::<Vec<_>>();
        let mut activities = keys
            .into_iter()
            .filter_map(|key| self.messages.remove(&key))
            .map(|binding| binding.activity)
            .collect::<Vec<_>>();
        activities.sort_unstable();
        self.pending_events
            .extend(
                activities
                    .into_iter()
                    .map(|activity| BackendEvent::ActivityFinished {
                        activity,
                        outcome: ActivityOutcome::Completed,
                    }),
            );
    }

    fn queue_tool_progress(&mut self, tool_id: &str, update: &Value) -> Result<(), BackendFailure> {
        let terminal = tool_terminal_outcome(update)?;
        let binding = self
            .tools
            .get_mut(tool_id)
            .expect("validated tool binding remains present");
        let mut output_changed = binding.result_activity.is_some()
            && (update.get("name").is_some() || update.get("title").is_some());
        for field in ["rawInput", "rawOutput", "content", "locations", "_meta"] {
            if let Some(value) = update.get(field) {
                binding.output[field] = value.clone();
                output_changed |= field != "rawInput" || binding.result_activity.is_some();
            }
        }
        let output = output_changed.then(|| tool_result_snapshot(tool_id, binding));
        if binding.file_change
            && let Some(output) = &output
        {
            let mut snapshot = String::new();
            if let Some(output) = ToolOutput::from_snapshot(output) {
                for block in output.content_blocks() {
                    if block.get("type").and_then(Value::as_str) != Some("diff") {
                        continue;
                    }
                    let (Some(text), Some(path)) = (
                        block.get("text").and_then(Value::as_str),
                        block
                            .get("source")
                            .and_then(|source| source.get("path"))
                            .and_then(Value::as_str),
                    ) else {
                        continue;
                    };
                    let path = serde_json::to_string(path).expect("string serialization");
                    let operation = if block["source"]["oldText"].is_null() {
                        "add"
                    } else {
                        "update"
                    };
                    snapshot.push_str(&format!("{operation}: {path}\n"));
                    snapshot.push_str(if text.is_empty() {
                        "No textual changes\n"
                    } else {
                        text
                    });
                }
            }
            if snapshot.is_empty() {
                snapshot = tool_identity_snapshot(&binding.identity, tool_id);
            }
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity: binding.activity,
                    update: ActivityUpdate::TextSnapshot(snapshot),
                });
        }

        let (call_activity, existing_result) = self
            .tools
            .get(tool_id)
            .map(|binding| (binding.activity, binding.result_activity))
            .expect("validated tool binding remains present");
        let needs_result = output.is_some() || terminal.is_some();
        let (result_activity, result_started) = match (needs_result, existing_result) {
            (false, _) => (None, false),
            (true, Some(activity)) => (Some(activity), false),
            (true, None) => {
                self.ensure_activity_capacity()?;
                let activity = self.next_activity(call_activity.turn())?;
                self.tools
                    .get_mut(tool_id)
                    .expect("validated tool binding remains present")
                    .result_activity = Some(activity);
                self.pending_events
                    .push_back(BackendEvent::ActivityStarted {
                        activity,
                        kind: ActivityKind::ToolResult,
                    });
                (Some(activity), true)
            },
        };
        if let Some(activity) = result_activity
            && let Some(output) = output.or_else(|| {
                result_started.then(|| {
                    tool_result_snapshot(
                        tool_id,
                        self.tools.get(tool_id).expect("validated tool binding"),
                    )
                })
            })
        {
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(output),
                });
        }
        if let Some(outcome) = terminal {
            let binding = self
                .tools
                .get_mut(tool_id)
                .expect("validated tool binding remains present");
            binding.finished = true;
            binding.output = json!({});
            if let Some(activity) = result_activity {
                self.pending_events
                    .push_back(BackendEvent::ActivityFinished {
                        activity,
                        outcome: outcome.clone(),
                    });
            }
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity: call_activity,
                    outcome,
                });
        }
        Ok(())
    }

    fn tool_call_update(&mut self, update: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        self.active_turn()?;
        let tool_id = identifier_at(update, "toolCallId")?.to_owned();
        let binding = self.tools.get(&tool_id).ok_or_else(|| {
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
        self.finish_anonymous_messages();
        let identity_snapshot = {
            let binding = self
                .tools
                .get_mut(&tool_id)
                .expect("validated tool binding remains present");
            merge_tool_identity(&mut binding.identity, update, &tool_id)
        };
        if let Some(snapshot) = identity_snapshot {
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(snapshot),
                });
        }
        self.queue_tool_progress(&tool_id, update)?;
        Ok(self.pending_events.pop_front())
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
        if self.read_only_review {
            self.client.reject(
                wire_id,
                -32000,
                "read-only delegated review does not accept permission requests",
            )?;
            return Err(protocol::protocol_failure(
                "Grok requested permission during a read-only delegated review",
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
        let tool_call_id = params
            .get("toolCall")
            .filter(|call| call.get("toolCallId").is_some())
            .map(|call| identifier_at(call, "toolCallId").map(str::to_owned))
            .transpose()?;
        let known = tool_call_id
            .as_ref()
            .and_then(|id| self.tools.get(id))
            .filter(|binding| binding.activity.turn() == turn && !binding.finished);
        let summary = params
            .get("toolCall")
            .and_then(permission_summary)
            .or_else(|| {
                let identity = &known?.identity;
                identity.title.clone().or_else(|| {
                    let call = &params["toolCall"];
                    let input = match call.get("rawInput") {
                        Some(input) => raw_input_summary(input),
                        None => identity.raw_input.clone(),
                    };
                    format_tool_summary(
                        Some(non_empty_text(call, "name").or(identity.name.as_deref())?),
                        Some(input.as_deref()?),
                    )
                })
            });
        let Some(summary) = summary else {
            self.client.respond(
                wire_id,
                json!({
                    "outcome": { "outcome": "selected", "optionId": reject_option }
                }),
            )?;
            return Err(protocol::protocol_failure(
                "Grok permission request was rejected because it has no actionable tool summary",
            ));
        };
        if options.len() > 64 {
            return Err(protocol::protocol_failure(
                "Grok permission request exceeds 64 choices",
            ));
        }
        let mut identifiers = Vec::new();
        let mut offered = Vec::new();
        let mut choices = Vec::new();
        let mut decline_choice = None;
        for (index, option) in options.iter().enumerate() {
            let id = identifier_at(option, "optionId")?.to_owned();
            if identifiers.contains(&id) {
                return Err(protocol::protocol_failure(
                    "Grok permission option IDs are not unique",
                ));
            }
            let kind = option.get("kind").and_then(Value::as_str).unwrap_or("");
            let description = match kind {
                "allow_once" => Some("Allow this operation once."),
                "reject_once" => Some("Reject this operation once."),
                "allow_always" => Some(
                    "Allow and ask the agent to remember this choice; scope is controlled by the agent.",
                ),
                "reject_always" => Some(
                    "Reject and ask the agent to remember this choice; scope is controlled by the agent.",
                ),
                _ => None,
            };
            if id == reject_option {
                decline_choice = Some(index as u32 + 1);
            }
            choices.push(ApprovalChoice {
                label: non_empty_text(option, "name")
                    .unwrap_or("Unnamed decision")
                    .to_owned(),
                description: description
                    .unwrap_or("Unsupported permission kind; this adapter cannot submit it.")
                    .to_owned(),
                enabled: description.is_some(),
            });
            offered.push(description.map(|_| id.clone()));
            identifiers.push(id);
        }
        let related_change = known
            .filter(|binding| binding.file_change)
            .map(|binding| binding.activity.activity_id().get().get());
        let mut plain_text = summary;
        if let Some(id) = params["toolCall"].get("toolCallId").and_then(Value::as_str) {
            plain_text.push_str(&format!("\n\nTool call: {id}"));
        }
        if let Some(arguments) = params["toolCall"]
            .get("rawInput")
            .or_else(|| known.and_then(|binding| binding.output.get("rawInput")))
        {
            plain_text.push_str(&format!("\n\nArguments:\n{arguments:#}"));
        }
        let mut display = json!({});
        for field in ["rawInput", "content", "locations"] {
            if let Some(value) = params["toolCall"].get(field) {
                display[field] = value.clone();
                if field != "rawInput" {
                    plain_text.push_str(&format!("\n\nReported {field}:\n{value:#}"));
                }
            }
        }
        let pending_display = (known.is_none()
            && tool_call_id.is_some()
            && display.as_object().is_some_and(|fields| !fields.is_empty()))
        .then(|| display.clone());
        let profile = ActivityApproval {
            related_change,
            plain_text,
            choices,
            decline_choice,
        };
        let fits_link = tool_call_id.is_none() || profile.related_change.is_some() || {
            let mut linked = profile.clone();
            linked.related_change = Some(u64::MAX);
            linked.to_snapshot().is_some()
        };
        let Some(summary) = profile.to_snapshot().filter(|_| fits_link) else {
            self.client.respond(
                wire_id,
                json!({"outcome":{"outcome":"selected","optionId":reject_option}}),
            )?;
            return Err(protocol::protocol_failure(
                "Grok permission details exceed the display limit",
            ));
        };
        let active_call = known.is_some();
        let wire_key = wire_key(&wire_id)?;
        if self.wire_approvals.contains_key(&wire_key) {
            return Err(protocol::protocol_failure(
                "duplicate Grok ACP permission request id",
            ));
        }
        self.finish_anonymous_messages();
        if active_call && let Some(id) = &tool_call_id {
            self.queue_tool_progress(id, &display)?;
        }
        self.ensure_activity_capacity()?;
        let activity = self.next_activity(turn)?;
        let request_id = self.next_request()?;
        let request = ActivityRequestRef::new(activity, request_id);
        self.approvals.insert(
            request,
            ApprovalBinding {
                wire_id,
                activity,
                allow_option,
                reject_option,
                offered,
                profile,
                tool_call_id,
                pending_display,
            },
        );
        self.wire_approvals.insert(wire_key, request);
        self.pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ApprovalRequest { request_id },
            });
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(summary),
            });
        Ok(self.pending_events.pop_front())
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
        let usage_receipt = prompt_usage_receipt(result, response_id)?;
        let activity_outcome = match &outcome {
            TurnOutcome::Completed => ActivityOutcome::Completed,
            TurnOutcome::Interrupted => ActivityOutcome::Interrupted,
            TurnOutcome::Failed(failure) => ActivityOutcome::Failed(failure.clone()),
        };
        let mut activities = self
            .messages
            .drain()
            .map(|(_, binding)| binding.activity)
            .chain(self.tools.drain().flat_map(|(_, binding)| {
                (!binding.finished)
                    .then_some(binding.activity)
                    .into_iter()
                    .chain(
                        (!binding.finished)
                            .then_some(binding.result_activity)
                            .flatten(),
                    )
            }))
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
        if let Some(receipt) = usage_receipt {
            self.queue_usage_activity(prompt.turn, receipt)?;
        }
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

    fn queue_usage_activity(
        &mut self,
        turn: TurnRef,
        receipt: Value,
    ) -> Result<(), BackendFailure> {
        let activity = self.next_activity(turn)?;
        self.pending_events
            .push_back(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            });
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(receipt.to_string()),
            });
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(())
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

fn prompt_usage_receipt(result: &Value, response_id: u64) -> Result<Option<Value>, BackendFailure> {
    if let Some(usage) = result.get("usage") {
        return standard_prompt_usage_receipt(usage, response_id).map(Some);
    }

    let Some(usage) = result.get("_meta").and_then(|meta| meta.get("usage")) else {
        return Ok(None);
    };
    grok_meta_prompt_usage_receipt(usage, response_id)
}

fn standard_prompt_usage_receipt(usage: &Value, response_id: u64) -> Result<Value, BackendFailure> {
    require_usage_object(usage, "usage")?;
    Ok(json!({
        "schema": "grok.acp-prompt-usage-receipt/v1",
        "source_profile": "grok.acp.prompt-response.usage/v1",
        "prompt_request_id": response_id,
        "usage": {
            "input_tokens": non_negative_usage_at(usage, "inputTokens")?,
            "output_tokens": non_negative_usage_at(usage, "outputTokens")?,
            "total_tokens": non_negative_usage_at(usage, "totalTokens")?,
            "reasoning_tokens": non_negative_usage_at(usage, "thoughtTokens")?,
            "cache_read_input_tokens": non_negative_usage_at(usage, "cachedReadTokens")?,
            "cache_write_input_tokens": non_negative_usage_at(usage, "cachedWriteTokens")?,
        }
    }))
}

fn grok_meta_prompt_usage_receipt(
    usage: &Value,
    response_id: u64,
) -> Result<Option<Value>, BackendFailure> {
    require_usage_object(usage, "_meta.usage")?;
    let incomplete = match usage.get("usageIsIncomplete") {
        None => false,
        Some(Value::Bool(incomplete)) => *incomplete,
        Some(_) => {
            return Err(protocol::protocol_failure(
                "Grok ACP prompt _meta.usage `usageIsIncomplete` is not a boolean",
            ));
        },
    };
    if incomplete {
        return Ok(None);
    }
    let input_tokens = non_negative_usage_at(usage, "inputTokens")?;
    let output_tokens = non_negative_usage_at(usage, "outputTokens")?;
    let total_tokens = non_negative_usage_at(usage, "totalTokens")?;
    let reasoning_tokens = non_negative_usage_at(usage, "reasoningTokens")?;
    let cache_read_input_tokens = non_negative_usage_at(usage, "cachedReadTokens")?;
    let cache_write_input_tokens = non_negative_usage_at(usage, "cacheCreationTokens")?;
    let model_calls = non_negative_usage_at(usage, "modelCalls")?;
    let num_turns = non_negative_usage_at(usage, "numTurns")?;
    Ok(Some(json!({
        "schema": "grok.acp-prompt-usage-receipt/v1alpha1",
        "source_profile": "grok.acp.prompt-response.meta-usage/v1",
        "prompt_request_id": response_id,
        "model_calls": model_calls,
        "num_turns": num_turns,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "total_tokens": total_tokens,
            "reasoning_tokens": reasoning_tokens,
            "cache_read_input_tokens": cache_read_input_tokens,
            "cache_write_input_tokens": cache_write_input_tokens,
        }
    })))
}

fn require_usage_object(usage: &Value, field: &'static str) -> Result<(), BackendFailure> {
    if usage.is_object() {
        Ok(())
    } else {
        Err(protocol::protocol_failure(format!(
            "Grok ACP prompt {field} is not an object"
        )))
    }
}

fn non_negative_usage_at(usage: &Value, field: &'static str) -> Result<u64, BackendFailure> {
    usage.get(field).and_then(Value::as_u64).ok_or_else(|| {
        protocol::protocol_failure(format!(
            "Grok ACP prompt usage `{field}` is not non-negative"
        ))
    })
}

fn activity_kind(kind: Option<&str>) -> ActivityKind {
    match kind {
        Some("edit" | "delete" | "move") => ActivityKind::FileChange,
        Some("think") => ActivityKind::ModelWork,
        _ => ActivityKind::ToolCall,
    }
}

fn tool_terminal_outcome(update: &Value) -> Result<Option<ActivityOutcome>, BackendFailure> {
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
    Ok(Some(outcome))
}

fn diff_content(item: &Value) -> Option<Value> {
    if item.get("type")?.as_str()? != "diff" {
        return None;
    }
    let path = item.get("path")?.as_str()?;
    let old = match item.get("oldText")? {
        Value::Null => None,
        Value::String(text) => Some(text.as_str()),
        _ => return None,
    };
    let new = item.get("newText")?.as_str()?;
    if old.unwrap_or_default().len().checked_add(new.len())? > 256 * 1024 {
        return None;
    }
    let quoted_path = serde_json::to_string(path).ok()?;
    let diff = TextDiff::configure()
        .timeout(Duration::from_millis(50))
        .diff_lines(old.unwrap_or_default(), new);
    let text = diff
        .unified_diff()
        .context_radius(3)
        .header(
            if old.is_none() {
                "/dev/null"
            } else {
                &quoted_path
            },
            &quoted_path,
        )
        .to_string();
    Some(json!({
        "type":"diff", "text":text,
        "title":format!("{} · {path}", if old.is_none() { "New file" } else { "File change" }),
        "source":item,
    }))
}

fn tool_result_snapshot(tool_id: &str, binding: &ToolBinding) -> String {
    let fields = &binding.output;
    let content = fields
        .get("content")
        .map(|content| match content.as_array() {
            Some(items) => Value::Array(
                items
                    .iter()
                    .map(|item| {
                        if let Some(diff) = diff_content(item) {
                            return diff;
                        }
                        if item.get("type").and_then(Value::as_str) == Some("content")
                            && item.as_object().is_some_and(|fields| fields.len() == 2)
                        {
                            item.get("content").cloned().unwrap_or_else(|| item.clone())
                        } else {
                            item.clone()
                        }
                    })
                    .collect(),
            ),
            None => content.clone(),
        });
    let mut result = fields.clone();
    let object = result
        .as_object_mut()
        .expect("retained tool output is an object");
    object.remove("content");
    object.remove("rawInput");
    let result = (!object.is_empty()).then_some(result);
    let mut parts = Vec::new();
    if let Some(items) = content.as_ref().and_then(Value::as_array) {
        for item in items {
            if item.get("type").and_then(Value::as_str) == Some("text")
                && let Some(text) = item.get("text").and_then(Value::as_str)
            {
                parts.push(text.to_owned());
            } else if item.get("type").and_then(Value::as_str) == Some("diff")
                && let (Some(title), Some(text)) = (
                    item.get("title").and_then(Value::as_str),
                    item.get("text").and_then(Value::as_str),
                )
            {
                parts.push(format!("{title}\n{text}"));
            } else {
                parts.push(format!("{item:#}"));
            }
        }
    } else if let Some(content) = &content {
        parts.push(format!("{content:#}"));
    }
    if let Some(result) = &result {
        parts.push(format!("{result:#}"));
    }
    let tool = binding.identity.name.as_deref().unwrap_or(tool_id);
    let mut plain_text = format!("{tool} · {tool_id}");
    if let Some(arguments) = fields.get("rawInput") {
        plain_text.push_str(&format!("\nArguments:\n{arguments:#}"));
    }
    plain_text.push_str(&format!("\n{}", parts.join("\n")));
    ToolOutput {
        tool: tool.to_owned(),
        server: None,
        arguments: fields.get("rawInput").cloned(),
        result,
        content_items: content,
        error: None,
        plain_text: plain_text.clone(),
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("{tool} · {tool_id}\n{fields:#}"))
}

fn tool_identity(update: &Value) -> ToolIdentity {
    ToolIdentity {
        title: non_empty_text(update, "title").map(str::to_owned),
        name: non_empty_text(update, "name").map(str::to_owned),
        raw_input: update.get("rawInput").and_then(raw_input_summary),
    }
}

fn merge_tool_identity(
    identity: &mut ToolIdentity,
    update: &Value,
    tool_id: &str,
) -> Option<String> {
    let before = tool_identity_snapshot(identity, tool_id);
    let observed = tool_identity(update);
    if identity.title.is_none() {
        identity.title = observed.title;
    }
    if identity.name.is_none() {
        identity.name = observed.name;
    }
    if identity.raw_input.is_none() {
        identity.raw_input = observed.raw_input;
    }
    let after = tool_identity_snapshot(identity, tool_id);
    (after != before).then_some(after)
}

fn tool_identity_snapshot(identity: &ToolIdentity, tool_id: &str) -> String {
    identity.title.clone().unwrap_or_else(|| {
        format_tool_summary(identity.name.as_deref(), identity.raw_input.as_deref())
            .unwrap_or_else(|| tool_id.to_owned())
    })
}

fn permission_summary(tool_call: &Value) -> Option<String> {
    non_empty_text(tool_call, "title")
        .map(str::to_owned)
        .or_else(|| {
            let name = non_empty_text(tool_call, "name")?;
            let raw_input = tool_call.get("rawInput").and_then(raw_input_summary)?;
            format_tool_summary(Some(name), Some(&raw_input))
        })
}

fn format_tool_summary(name: Option<&str>, raw_input: Option<&str>) -> Option<String> {
    match (name, raw_input) {
        (Some(name), Some(input)) => Some(format!("{name}: {input}")),
        (Some(name), None) => Some(name.to_owned()),
        (None, Some(input)) => Some(input.to_string()),
        (None, None) => None,
    }
}

fn raw_input_summary(value: &Value) -> Option<String> {
    meaningful_raw_input(value).then(|| match value {
        Value::String(input) => input.trim().to_owned(),
        value => value.to_string(),
    })
}

fn non_empty_text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn meaningful_raw_input(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(input) => !input.trim().is_empty(),
        Value::Array(items) => items.iter().any(meaningful_raw_input),
        Value::Object(fields) => fields.values().any(meaningful_raw_input),
        Value::Bool(_) | Value::Number(_) => true,
    }
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

fn reasoning_snapshot(content: Value) -> Result<String, BackendFailure> {
    ActivityReasoning { content }
        .to_snapshot()
        .ok_or_else(|| protocol::protocol_failure("Grok reasoning exceeds presentation limit"))
}
