use std::iter;

use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityDocument, ActivityKind, ActivityNotice, ActivityOutcome, ActivityPlan, ActivityUpdate,
    BackendEvent, BackendFailure, BackendFailureKind, BackendOutcomeEvidence, BackendPoll, Failure,
    NoticeLevel, PlanStep, PlanStepStatus, ToolOutput, TurnOutcome,
};

use super::{
    super::state::{Backend, ItemBinding, RequestKind},
    requests::wire_key,
    snapshots::{
        activity_kind, checked_command_snapshot, command_plain_text, compaction_snapshot,
        file_change_snapshot, item_text_snapshot, optional_non_negative_at, proposed_plan_snapshot,
        reasoning_snapshot, token_usage_breakdown_at, value_at,
    },
};
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

    fn close_connection(
        &mut self,
        terminal: Result<(), BackendFailure>,
    ) -> Result<BackendPoll, BackendFailure> {
        // Deliver interview receipts before the runtime's terminal failure closes the turn.
        // Never report recorded local answers as submitted after transport loss.
        let mut requests = self.requests.keys().copied().collect::<Vec<_>>();
        requests.sort_unstable_by_key(|request| request.activity());
        let mut events = Vec::new();
        for request in requests {
            let summaries = self.interview_summary_events(
                request,
                "Connection to Codex ended before submission completed.",
            )?;
            let binding = &self.requests[&request];
            events.push(BackendEvent::ActivityFinished {
                activity: binding.request_activity,
                outcome: if binding.responded {
                    ActivityOutcome::Completed
                } else {
                    ActivityOutcome::Interrupted
                },
            });
            events.extend(summaries);
        }
        self.requests.clear();
        self.wire_requests.clear();
        self.pending_events.extend(events);
        self.terminal_poll = Some(terminal.clone());
        match self.pending_events.pop_front() {
            Some(event) => Ok(BackendPoll::Event(event)),
            None => terminal.map(|()| BackendPoll::Closed),
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
        let proposed_plan = if item_type == "plan" {
            let text = protocol::string_at(params, &["item", "text"])?;
            proposed_plan_snapshot(text)?;
            Some(text.to_owned())
        } else {
            None
        };
        let command = (item_type == "commandExecution").then(|| params["item"].clone());
        if let Some(command) = &command {
            checked_command_snapshot(command)?;
        }
        let activity = self.next_activity(turn)?;
        if self
            .items
            .insert(
                item_id.clone(),
                ItemBinding {
                    activity,
                    proposed_plan,
                    command,
                    public_summary: (item_type == "reasoning").then(|| {
                        params
                            .pointer("/item/summary")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .enumerate()
                            .filter_map(|(index, part)| {
                                part.as_str().map(|text| (index as u64, text.to_owned()))
                            })
                            .collect()
                    }),
                },
            )
            .is_some()
        {
            return Err(protocol::protocol_failure(format!(
                "duplicate Codex item `{item_id}`"
            )));
        }
        if item_type == "commandExecution" {
            self.terminal_commands.insert(
                item_id.clone(),
                (
                    turn,
                    params
                        .pointer("/item/command")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                ),
            );
        }
        if (matches!(
            kind,
            ActivityKind::ToolCall | ActivityKind::ToolResult | ActivityKind::FileChange
        ) || matches!(
            item_type,
            "reasoning"
                | "contextCompaction"
                | "plan"
                | "enteredReviewMode"
                | "exitedReviewMode"
                | "subAgentActivity"
                | "hookPrompt"
        )) && (item_type != "commandExecution" || command_plain_text(&params["item"]).is_some())
            && let Some(mut snapshot) = item_text_snapshot(params)
        {
            if item_type == "contextCompaction" {
                snapshot = compaction_snapshot(false);
            }
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(snapshot),
                });
        }
        if item_type == "fileChange" && file_change_snapshot(&params["item"]).is_some() {
            self.link_file_approvals(&item_id, activity);
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
        if item_type == "plan" {
            proposed_plan_snapshot(protocol::string_at(params, &["item", "text"])?)?;
        }
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
        let command_snapshot =
            if let Some(command) = binding.and_then(|binding| binding.command.as_ref()) {
                let mut final_item = command.clone();
                final_item["status"] = params["item"]
                    .get("status")
                    .cloned()
                    .unwrap_or_else(|| Value::String("completed".to_owned()));
                if let Some(fields) = params["item"].as_object() {
                    final_item
                        .as_object_mut()
                        .expect("command item is an object")
                        .extend(
                            fields
                                .iter()
                                .filter(|(key, value)| {
                                    !value.is_null()
                                        || !matches!(
                                            key.as_str(),
                                            "command" | "cwd" | "aggregatedOutput"
                                        )
                                })
                                .map(|(key, value)| (key.clone(), value.clone())),
                        );
                }
                Some(checked_command_snapshot(&final_item)?)
            } else {
                None
            };
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
            "interrupted" if item_type == "collabAgentToolCall" => ActivityOutcome::Interrupted,
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
        if let Some(snapshot) = command_snapshot.or_else(|| item_text_snapshot(params)) {
            self.pending_events.push_back(finished);
            if item_type == "fileChange" && file_change_snapshot(&params["item"]).is_some() {
                self.link_file_approvals(item_id, binding.activity);
            }
            return Ok(Some(BackendEvent::ActivityUpdated {
                activity: binding.activity,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }));
        }
        Ok(Some(finished))
    }

    fn item_delta(&mut self, params: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
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
        if let Some(command) = &binding.command {
            let activity = binding.activity;
            let mut next = command.clone();
            let output = next
                .get("aggregatedOutput")
                .and_then(Value::as_str)
                .unwrap_or("");
            if output.len().saturating_add(delta.len()) > 16 * 1024 * 1024 {
                return Err(protocol::protocol_failure(
                    "command output exceeds output profile limit",
                ));
            }
            next["aggregatedOutput"] = Value::String(format!("{output}{delta}"));
            let snapshot = checked_command_snapshot(&next)?;
            self.items
                .get_mut(item_id)
                .expect("validated command binding")
                .command = Some(next);
            return Ok(Some(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(snapshot),
            }));
        }
        Ok(Some(BackendEvent::ActivityUpdated {
            activity: binding.activity,
            update: ActivityUpdate::TextDelta(delta.to_owned()),
        }))
    }

    fn terminal_interaction(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let binding = self.wire_turns.get(wire_turn).copied().ok_or_else(|| {
            protocol::protocol_failure("terminal interaction targets an unknown turn")
        })?;
        if binding.finished {
            return Ok(None);
        }
        let item_id = protocol::string_at(params, &["itemId"])?;
        let (owner, command) = self.terminal_commands.get(item_id).ok_or_else(|| {
            protocol::protocol_failure("terminal interaction targets an unknown command")
        })?;
        if *owner != binding.turn {
            return Err(protocol::protocol_failure(
                "terminal interaction changed turn",
            ));
        }
        let process_id = protocol::string_at(params, &["processId"])?;
        let input = protocol::string_at(params, &["stdin"])?;
        let mut source = format!("Process: {process_id}");
        if let Some(command) = command {
            source.push_str(&format!("\nCommand: {command}"));
        }
        if !input.is_empty() {
            source.push_str(&format!("\nInput:\n{input}"));
        }
        // Longer fences keep terminal input literal even when it contains Markdown fences.
        let longest = source.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat(longest.saturating_add(1).max(3));
        let snapshot = ActivityDocument {
            title: if input.is_empty() {
                "Waited for background terminal"
            } else {
                "Terminal input sent"
            }
            .to_owned(),
            markdown: format!("{fence}text\n{source}\n{fence}"),
        }
        .to_snapshot()
        .ok_or_else(|| {
            protocol::protocol_failure("terminal interaction exceeds output profile limit")
        })?;
        let activity = self.next_activity(binding.turn)?;
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(snapshot),
            });
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }))
    }

    fn proposed_plan_delta(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta(delta),
        }) = self.item_delta(params)?
        else {
            unreachable!("validated delta")
        };
        let item_id = protocol::string_at(params, &["itemId"])?;
        let source = self
            .items
            .get_mut(item_id)
            .and_then(|binding| binding.proposed_plan.as_mut())
            .ok_or_else(|| protocol::protocol_failure("plan delta targets a non-plan item"))?;
        let mut next = source.clone();
        next.push_str(&delta);
        let snapshot = proposed_plan_snapshot(&next)?;
        *source = next;
        Ok(Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(snapshot),
        }))
    }

    fn reasoning_summary_delta(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        // Reuse item/thread/turn validation before mutating any retained summary.
        let Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta(delta),
        }) = self.item_delta(params)?
        else {
            unreachable!("item_delta returns an update or a protocol error");
        };
        let index = params
            .get("summaryIndex")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                protocol::protocol_failure("Codex summaryIndex must be a non-negative integer")
            })?;
        let item_id = protocol::string_at(params, &["itemId"])?;
        let summary = self
            .items
            .get_mut(item_id)
            .and_then(|binding| binding.public_summary.as_mut())
            .ok_or_else(|| {
                protocol::protocol_failure("summary delta targets a non-reasoning item")
            })?;
        // Sparse indices must not allocate placeholder parts. Snapshots preserve part
        // order even when deltas for separate public-summary parts are interleaved.
        summary.entry(index).or_default().push_str(&delta);
        let text = summary
            .values()
            .filter(|part| !part.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(Some(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(reasoning_snapshot(text).ok_or_else(|| {
                protocol::protocol_failure(
                    "Codex public reasoning summary exceeds output profile limit",
                )
            })?),
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
        if self
            .wire_turns
            .get(wire_turn)
            .is_some_and(|binding| binding.finished)
        {
            return Ok(None);
        }
        let duration_ms = match params.pointer("/turn/durationMs") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_i64()
                    .filter(|duration| *duration >= 0)
                    .ok_or_else(|| {
                        protocol::protocol_failure(
                            "Codex turn durationMs must be a non-negative int64",
                        )
                    })? as u64,
            ),
        };
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
        let turn_finished = if outcome == TurnOutcome::Completed {
            BackendEvent::ResumableTurnFinished {
                turn,
                evidence: BackendOutcomeEvidence::without_identity(),
            }
        } else {
            BackendEvent::TurnFinished {
                turn,
                outcome: outcome.clone(),
            }
        };
        let mut completion_events = Vec::new();
        if let Some(duration_ms) = duration_ms {
            let activity = self.next_activity(turn)?;
            let seconds = duration_ms / 1000;
            let elapsed = if seconds >= 60 {
                format!(
                    "{}m {}.{:03}s",
                    seconds / 60,
                    seconds % 60,
                    duration_ms % 1000
                )
            } else {
                format!("{}.{:03}s", seconds, duration_ms % 1000)
            };
            let notice = ActivityNotice {
                level: if status == "failed" {
                    NoticeLevel::Warning
                } else {
                    NoticeLevel::Info
                },
                title: format!("Turn {status}"),
                message: format!("Duration: {elapsed} ({duration_ms} ms, reported by Codex)"),
            }
            .to_snapshot()
            .expect("bounded duration notice");
            completion_events.extend([
                BackendEvent::ActivityStarted {
                    activity,
                    kind: ActivityKind::ModelWork,
                },
                BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(notice),
                },
                BackendEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                },
            ]);
        }
        self.wire_turns
            .get_mut(wire_turn)
            .expect("completed turn was resolved above")
            .finished = true;
        self.terminal_commands
            .retain(|_, (owner, _)| *owner != turn);
        self.file_changes.retain(|(owner, _), _| *owner != turn);
        let plan = self.plans.remove(&turn);
        let diff = self.turn_diffs.remove(&turn);
        if outcome != TurnOutcome::Interrupted {
            let mut interviews = self
                .requests
                .iter()
                .filter(|(_, binding)| {
                    binding.request_activity.turn() == turn
                        && matches!(binding.kind, RequestKind::Input(_))
                })
                .map(|(request, binding)| (*request, binding.request_activity))
                .collect::<Vec<_>>();
            interviews.sort_unstable_by_key(|(_, activity)| *activity);
            for (request, activity) in interviews {
                let summaries = self.interview_summary_events(
                    request,
                    if status == "failed" {
                        "Turn failed."
                    } else {
                        "Turn ended."
                    },
                )?;
                let binding = self
                    .requests
                    .remove(&request)
                    .expect("collected interview exists");
                self.wire_requests.remove(&wire_key(&binding.wire_id)?);
                let request_outcome = if binding.responded {
                    ActivityOutcome::Completed
                } else {
                    match &outcome {
                        TurnOutcome::Failed(failure) => ActivityOutcome::Failed(failure.clone()),
                        _ => ActivityOutcome::Interrupted,
                    }
                };
                self.pending_events
                    .push_back(BackendEvent::ActivityFinished {
                        activity,
                        outcome: request_outcome,
                    });
                self.pending_events.extend(summaries);
            }
            for activity in plan.into_iter().chain(diff) {
                let outcome = match &outcome {
                    TurnOutcome::Completed => ActivityOutcome::Completed,
                    TurnOutcome::Failed(failure) => ActivityOutcome::Failed(failure.clone()),
                    TurnOutcome::Interrupted => unreachable!("handled below"),
                };
                self.pending_events
                    .push_back(BackendEvent::ActivityFinished { activity, outcome });
            }
            self.pending_events.extend(completion_events);
            self.pending_events.push_back(turn_finished);
            return Ok(self.pending_events.pop_front());
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
        let mut interrupted_requests = self
            .requests
            .iter()
            .filter(|(_, binding)| binding.request_activity.turn() == turn)
            .map(|(request, binding)| (*request, binding.request_activity))
            .collect::<Vec<_>>();

        let mut interrupted_activities =
            Vec::with_capacity(interrupted_items.len() + interrupted_requests.len());
        interrupted_activities.extend(plan);
        interrupted_activities.extend(diff);
        for (item_id, activity) in interrupted_items {
            self.items.remove(&item_id);
            interrupted_activities.push(activity);
        }
        interrupted_requests.sort_unstable_by_key(|(_, activity)| *activity);
        for (request, activity) in interrupted_requests {
            completion_events.extend(self.interview_summary_events(request, "Turn interrupted.")?);
            let binding = self
                .requests
                .remove(&request)
                .expect("collected request binding must still exist");
            let key = wire_key(&binding.wire_id)?;
            self.wire_requests.remove(&key);
            interrupted_activities.push(activity);
        }
        interrupted_activities.sort_unstable();

        let mut terminal_events = interrupted_activities
            .into_iter()
            .map(|activity| BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Interrupted,
            })
            .chain(completion_events)
            .chain(iter::once(turn_finished));
        let first = terminal_events
            .next()
            .expect("an interrupted Turn always has its own terminal event");
        self.pending_events.extend(terminal_events);
        Ok(Some(first))
    }

    fn token_usage_updated(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let Some(turn) = self.wire_turns.get(wire_turn).map(|binding| binding.turn) else {
            // thread/resume can replay the persisted usage snapshot for a historical
            // Codex Turn. That Turn has no trustworthy Yo Turn binding in this process,
            // so keep the downstream boundary exact instead of inventing attribution.
            return Ok(None);
        };
        let token_usage = value_at(params, &["tokenUsage"], "token usage")?;
        let last = token_usage_breakdown_at(token_usage, "last")?;
        let total = token_usage_breakdown_at(token_usage, "total")?;
        let model_context_window =
            optional_non_negative_at(token_usage, "modelContextWindow", "model context window")?;
        let receipt = json!({
            "schema": "codex.app-server-token-usage-receipt/v1",
            "source_profile": "codex.app-server.thread-token-usage-updated/v1",
            "turn_id": wire_turn,
            "usage": last.to_json(),
            "thread_total": total.to_json(),
            "model_context_window": model_context_window,
        });
        self.usage_activity(turn, receipt)
    }

    fn usage_activity(
        &mut self,
        turn: yo_core::TurnRef,
        receipt: Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let activity = self.next_activity(turn)?;
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
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }))
    }

    fn turn_diff_updated(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let binding = self
            .wire_turns
            .get(wire_turn)
            .copied()
            .ok_or_else(|| protocol::protocol_failure("diff targets an unknown turn"))?;
        if binding.finished {
            return Ok(None);
        }
        let diff = protocol::string_at(params, &["diff"])?;
        let text = format!(
            "Turn aggregate diff\n{}",
            if diff.is_empty() {
                "No remaining changes reported for this turn."
            } else {
                diff
            }
        );
        if text.len() > ToolOutput::MAX_SNAPSHOT_BYTES {
            return Err(protocol::protocol_failure(
                "turn diff exceeds output presentation limit",
            ));
        }
        if let Some(&activity) = self.turn_diffs.get(&binding.turn) {
            return Ok(Some(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }));
        }
        let activity = self.next_activity(binding.turn)?;
        self.turn_diffs.insert(binding.turn, activity);
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            });
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::FileChange,
        }))
    }

    fn model_rerouted(&mut self, params: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let binding =
            self.wire_turns.get(wire_turn).copied().ok_or_else(|| {
                protocol::protocol_failure("model reroute targets an unknown turn")
            })?;
        if binding.finished {
            return Ok(None);
        }
        let from = protocol::string_at(params, &["fromModel"])?;
        let to = protocol::string_at(params, &["toModel"])?;
        let reason = protocol::string_at(params, &["reason"])?;
        let notice = ActivityNotice {
            title: "Model rerouted".to_owned(),
            message: format!(
                "Codex reported a model change.\nFrom: {from}\nTo: {to}\nReason: {reason}"
            ),
            level: NoticeLevel::Warning,
        }
        .to_snapshot()
        .ok_or_else(|| {
            protocol::protocol_failure("model reroute notice exceeds the presentation bound")
        })?;
        let activity = self.next_activity(binding.turn)?;
        self.pending_events
            .push_back(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(notice),
            });
        self.pending_events
            .push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ModelWork,
        }))
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
        let binding = self.wire_turns.get(wire_turn).copied().ok_or_else(|| {
            protocol::protocol_failure(format!("error targets unknown Codex Turn `{wire_turn}`"))
        })?;
        if binding.finished {
            return Ok(None);
        }
        let will_retry = match params.get("willRetry") {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return Err(protocol::protocol_failure(
                    "Codex willRetry must be boolean",
                ));
            },
        };
        if will_retry {
            // A transient error must never become the later terminal failure reason.
            self.turn_errors.remove(wire_turn);
            let activity = self.next_activity(binding.turn)?;
            let notice = ActivityNotice {
                title: "Retry announced".to_owned(),
                message: format!("{message}\nCodex will retry."),
                level: NoticeLevel::Warning,
            };
            let text = notice
                .to_snapshot()
                .unwrap_or_else(|| format!("{}\n{}", notice.title, notice.message));
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(text),
                });
            self.pending_events
                .push_back(BackendEvent::ActivityFinished {
                    activity,
                    outcome: ActivityOutcome::Completed,
                });
            return Ok(Some(BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            }));
        }
        self.turn_errors.insert(wire_turn.to_owned(), message);
        Ok(None)
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

impl<P: JsonMessagePeer> Backend<P> {
    fn plan_updated(&mut self, params: &Value) -> Result<Option<BackendEvent>, BackendFailure> {
        self.validate_thread(params)?;
        let wire_turn = protocol::string_at(params, &["turnId"])?;
        let binding = self
            .wire_turns
            .get(wire_turn)
            .copied()
            .ok_or_else(|| protocol::protocol_failure("plan targets an unknown turn"))?;
        if binding.finished {
            return Ok(None);
        }
        let steps = params
            .get("plan")
            .and_then(Value::as_array)
            .ok_or_else(|| protocol::protocol_failure("plan steps must be an array"))?;
        let mut plan_steps = Vec::with_capacity(steps.len());
        for entry in steps {
            let text = protocol::string_at(entry, &["step"])?.to_owned();
            let status = match protocol::string_at(entry, &["status"])? {
                "completed" => PlanStepStatus::Completed,
                "inProgress" => PlanStepStatus::InProgress,
                "pending" => PlanStepStatus::Pending,
                _ => {
                    return Err(protocol::protocol_failure(
                        "plan step has an unknown status",
                    ));
                },
            };
            plan_steps.push(PlanStep { text, status });
        }
        let explanation = match params.get("explanation") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            _ => return Err(protocol::protocol_failure("plan explanation must be text")),
        };
        let text = ActivityPlan {
            explanation,
            steps: plan_steps,
        }
        .to_snapshot()
        .ok_or_else(|| protocol::protocol_failure("plan exceeds output profile limit"))?;
        if let Some(&activity) = self.plans.get(&binding.turn) {
            return Ok(Some(BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            }));
        }
        let activity = self.next_activity(binding.turn)?;
        self.plans.insert(binding.turn, activity);
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
}
