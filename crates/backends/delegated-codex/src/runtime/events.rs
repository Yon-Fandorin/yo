use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
};

use serde_json::{Map, Value, json, to_string_pretty};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityApproval, ActivityDocument, ActivityKind, ActivityNotice, ActivityOutcome,
    ActivityPlan, ActivityQuestion, ActivityRef, ActivityRequestRef, ActivitySummary,
    ActivityUpdate, ApprovalChoice, BackendEvent, BackendFailure, BackendFailureKind,
    BackendOutcomeEvidence, BackendPoll, Failure, NoticeLevel, PlanStep, PlanStepStatus,
    QuestionChoice, SummaryKind, ToolOutput, TurnOutcome,
};

use super::{Backend, InputQuestion, InputQuestions, ItemBinding, RequestBinding, RequestKind};
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
            .chain(std::iter::once(turn_finished));
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

    fn map_server_request(
        &mut self,
        wire_id: Value,
        method: &str,
        params: Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        if !matches!(
            method,
            "item/commandExecution/requestApproval"
                | "item/fileChange/requestApproval"
                | "item/tool/requestUserInput"
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
                protocol::protocol_failure(format!("request targets unknown Turn `{wire_turn}`"))
            })?;
        let is_approval = method != "item/tool/requestUserInput";
        let approval = if is_approval {
            match approval_summary(method, &params) {
                Ok(text) => Some(text),
                Err(error) => {
                    self.client.reject(
                        wire_id,
                        -32602,
                        "approval context is invalid or exceeds the display limit",
                    )?;
                    return Err(error);
                },
            }
        } else {
            None
        };
        let kind = if !is_approval {
            match InputQuestions::parse(&params) {
                Ok(questions) => RequestKind::Input(questions),
                Err(error) => {
                    self.client.reject(
                        wire_id,
                        -32602,
                        "user-input questions are invalid or require unsupported secret input",
                    )?;
                    return Err(error);
                },
            }
        } else {
            let (offered, explicit) = match approval_decisions(method, &params) {
                Ok(decisions) => decisions,
                Err(error) => {
                    self.client
                        .reject(wire_id, -32602, "approval decisions are invalid")?;
                    return Err(error);
                },
            };
            RequestKind::Approval {
                offered,
                explicit,
                command: method == "item/commandExecution/requestApproval",
            }
        };
        let approval = match (&kind, approval) {
            (
                RequestKind::Approval {
                    offered, command, ..
                },
                Some(plain_text),
            ) => {
                let choices = offered.iter().map(|value| approval_choice(value, *command).unwrap_or(ApprovalChoice {
                    label: "Unsupported decision".to_owned(),
                    description: "Review the reported decision in request history; this adapter cannot submit it.".to_owned(),
                    enabled: false,
                })).collect();
                let decline_choice = offered
                    .iter()
                    .position(|value| value.as_str() == Some("decline"))
                    .or_else(|| {
                        offered
                            .iter()
                            .position(|value| value.as_str() == Some("cancel"))
                    })
                    .and_then(|index| u32::try_from(index + 1).ok());
                let profile = ActivityApproval {
                    related_change: (method == "item/fileChange/requestApproval")
                        .then(|| params.get("itemId").and_then(Value::as_str))
                        .flatten()
                        .and_then(|id| self.file_changes.get(&(turn, id.to_owned())))
                        .map(|activity| activity.activity_id().get().get()),
                    plain_text,
                    choices,
                    decline_choice,
                };
                let reserve_link = method == "item/fileChange/requestApproval"
                    && profile.related_change.is_none()
                    && params.get("itemId").is_some_and(Value::is_string);
                let fits_later_link = !reserve_link || {
                    let mut linked = profile.clone();
                    linked.related_change = Some(u64::MAX);
                    linked.to_snapshot().is_some()
                };
                match profile.to_snapshot().filter(|_| fits_later_link) {
                    Some(text) => Some(text),
                    None => {
                        self.client.reject(
                            wire_id,
                            -32602,
                            "approval choices exceed the display limit",
                        )?;
                        return Err(protocol::protocol_failure(
                            "approval choices exceed the display limit",
                        ));
                    },
                }
            },
            (_, text) => text,
        };
        let activity = self.next_activity(turn)?;
        let request_id = self.next_request()?;
        let request = ActivityRequestRef::new(activity, request_id);
        let wire_key = wire_key(&wire_id)?;
        if self.wire_requests.contains_key(&wire_key) {
            return Err(protocol::protocol_failure("duplicate Codex request id"));
        }
        let file_approval = (method == "item/fileChange/requestApproval")
            .then(|| {
                let item_id = params.get("itemId")?.as_str()?.to_owned();
                let profile = ActivityApproval::from_snapshot(approval.as_deref()?)?;
                profile
                    .related_change
                    .is_none()
                    .then_some((item_id, profile))
            })
            .flatten();
        let (activity_kind, summary) = match &kind {
            RequestKind::Approval { .. } => {
                (ActivityKind::ApprovalRequest { request_id }, approval)
            },
            RequestKind::Input(questions) => (
                ActivityKind::UserInputRequest { request_id },
                Some(questions.prompt()),
            ),
        };
        self.requests.insert(
            request,
            RequestBinding {
                file_approval,
                wire_id,
                request_activity: activity,
                kind,
                responded: false,
            },
        );
        self.wire_requests.insert(wire_key, request);
        if let Some(summary) = summary {
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: ActivityUpdate::TextSnapshot(summary),
                });
        }
        Ok(Some(BackendEvent::ActivityStarted {
            activity,
            kind: activity_kind,
        }))
    }

    fn link_file_approvals(&mut self, item_id: &str, activity: ActivityRef) {
        self.file_changes
            .insert((activity.turn(), item_id.to_owned()), activity);
        let mut pending = self
            .requests
            .iter()
            .filter(|(_, binding)| {
                !binding.responded
                    && binding.request_activity.turn() == activity.turn()
                    && binding
                        .file_approval
                        .as_ref()
                        .is_some_and(|(id, _)| id == item_id)
            })
            .map(|(request, _)| *request)
            .collect::<Vec<_>>();
        pending.sort_unstable();
        for request in pending {
            let (_, mut profile) = self
                .requests
                .get_mut(&request)
                .expect("collected request exists")
                .file_approval
                .take()
                .expect("collected file approval exists");
            profile.related_change = Some(activity.activity_id().get().get());
            // Admission reserved the largest possible related activity ID encoding.
            let snapshot = profile
                .to_snapshot()
                .expect("file approval reserved its link bytes");
            self.pending_events
                .push_back(BackendEvent::ActivityUpdated {
                    activity: request.activity(),
                    update: ActivityUpdate::TextSnapshot(snapshot),
                });
        }
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
        let Some(request) = self.wire_requests.get(&key).copied() else {
            return Ok(None);
        };
        let summaries =
            self.interview_summary_events(request, "Question request closed by the agent.")?;
        self.wire_requests.remove(&key);
        let binding = self.requests.remove(&request).ok_or_else(|| {
            protocol::protocol_failure("resolved request lost its request binding")
        })?;
        self.pending_events.extend(summaries);
        Ok(Some(BackendEvent::ActivityFinished {
            activity: binding.request_activity,
            outcome: if binding.responded {
                ActivityOutcome::Completed
            } else {
                ActivityOutcome::Interrupted
            },
        }))
    }

    fn interview_summary_events(
        &mut self,
        request: ActivityRequestRef,
        reason: &str,
    ) -> Result<Vec<BackendEvent>, BackendFailure> {
        let Some(binding) = self
            .requests
            .get(&request)
            .filter(|binding| !binding.responded)
        else {
            return Ok(Vec::new());
        };
        let RequestKind::Input(questions) = &binding.kind else {
            return Ok(Vec::new());
        };
        let text = questions.incomplete_notice(reason);
        let activity = self.next_activity(request.activity().turn())?;
        Ok(vec![
            BackendEvent::ActivityStarted {
                activity,
                kind: ActivityKind::ModelWork,
            },
            BackendEvent::ActivityUpdated {
                activity,
                update: ActivityUpdate::TextSnapshot(text),
            },
            BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Completed,
            },
        ])
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

#[derive(Clone, Copy)]
struct TokenUsageBreakdown {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    reasoning_tokens: u64,
    cache_read_input_tokens: u64,
    cache_write_input_tokens: u64,
}

impl TokenUsageBreakdown {
    fn to_json(self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
            "reasoning_tokens": self.reasoning_tokens,
            "cache_read_input_tokens": self.cache_read_input_tokens,
            "cache_write_input_tokens": self.cache_write_input_tokens,
        })
    }
}

fn token_usage_breakdown_at(
    value: &Value,
    field: &'static str,
) -> Result<TokenUsageBreakdown, BackendFailure> {
    let value = value_at(value, &[field], "token usage breakdown")?;
    Ok(TokenUsageBreakdown {
        input_tokens: non_negative_at(value, "inputTokens", "input tokens")?,
        output_tokens: non_negative_at(value, "outputTokens", "output tokens")?,
        total_tokens: non_negative_at(value, "totalTokens", "total tokens")?,
        reasoning_tokens: non_negative_at(
            value,
            "reasoningOutputTokens",
            "reasoning output tokens",
        )?,
        cache_read_input_tokens: non_negative_at(
            value,
            "cachedInputTokens",
            "cached input tokens",
        )?,
        cache_write_input_tokens: optional_non_negative_at(
            value,
            "cacheWriteInputTokens",
            "cache write input tokens",
        )?
        .unwrap_or(0),
    })
}

fn value_at<'a>(
    value: &'a Value,
    path: &[&str],
    label: &'static str,
) -> Result<&'a Value, BackendFailure> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment).ok_or_else(|| {
            protocol::protocol_failure(format!("Codex message is missing {label}"))
        })?;
    }
    if !current.is_object() {
        return Err(protocol::protocol_failure(format!(
            "Codex {label} is not an object"
        )));
    }
    Ok(current)
}

fn non_negative_at(
    value: &Value,
    field: &'static str,
    label: &'static str,
) -> Result<u64, BackendFailure> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| protocol::protocol_failure(format!("Codex {label} is not non-negative")))
}

fn optional_non_negative_at(
    value: &Value,
    field: &'static str,
    label: &'static str,
) -> Result<Option<u64>, BackendFailure> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            protocol::protocol_failure(format!("Codex {label} is not non-negative"))
        }),
    }
}

fn activity_kind(item_type: &str) -> Option<ActivityKind> {
    match item_type {
        "agentMessage" => Some(ActivityKind::AgentMessage),
        "functionCallOutput" => Some(ActivityKind::ToolResult),
        "reasoning" | "plan" | "contextCompaction" | "enteredReviewMode" | "exitedReviewMode"
        | "subAgentActivity" | "hookPrompt" => Some(ActivityKind::ModelWork),
        "commandExecution"
        | "mcpToolCall"
        | "dynamicToolCall"
        | "webSearch"
        | "imageGeneration"
        | "imageView"
        | "collabAgentToolCall"
        | "sleep" => Some(ActivityKind::ToolCall),
        "fileChange" => Some(ActivityKind::FileChange),
        _ => None,
    }
}

pub(super) fn wire_key(value: &Value) -> Result<String, BackendFailure> {
    serde_json::to_string(value)
        .map_err(|error| protocol::protocol_failure(format!("invalid Codex request id: {error}")))
}

fn item_text_snapshot(params: &Value) -> Option<String> {
    let item = params.get("item")?;
    match item.get("type")?.as_str()? {
        "agentMessage" => item.get("text")?.as_str().map(str::to_owned),
        "functionCallOutput" => Some(function_output_snapshot(item)),
        "plan" => proposed_plan_snapshot(item.get("text")?.as_str()?).ok(),
        "enteredReviewMode" => Some(review_snapshot(item, false)),
        "exitedReviewMode" => Some(review_snapshot(item, true)),
        "commandExecution" => command_snapshot(item),
        "imageGeneration" => Some(image_generation_snapshot(item)),
        "imageView" => Some(image_view_snapshot(item)),
        "collabAgentToolCall" => Some(collab_tool_snapshot(item)),
        "subAgentActivity" => Some(sub_agent_activity_snapshot(item)),
        "hookPrompt" => Some(hook_context_snapshot(item)),
        "sleep" => Some(sleep_snapshot(item)),
        "fileChange" => file_change_snapshot(item),
        "mcpToolCall" | "dynamicToolCall" => tool_snapshot(item),
        "reasoning" => {
            // Only the host's public summary belongs in the conversation.
            // The separate raw content field is not a fallback for absent summaries.
            let summary = item
                .get("summary")?
                .as_array()?
                .iter()
                .map(Value::as_str)
                .collect::<Option<Vec<_>>>()?
                .join("\n\n");
            reasoning_snapshot(summary)
        },
        "webSearch" => Some(web_search_snapshot(item)),
        "contextCompaction" => Some(compaction_snapshot(true)),
        _ => None,
    }
}

fn hook_context_snapshot(item: &Value) -> String {
    let fallback = || format!("Hook context\n{item:#}");
    let Some(fragments) = item.get("fragments").and_then(Value::as_array) else {
        return fallback();
    };
    let mut parts = Vec::new();
    for fragment in fragments {
        let (Some(id), Some(text)) = (
            fragment.get("hookRunId").and_then(Value::as_str),
            fragment.get("text").and_then(Value::as_str),
        ) else {
            return fallback();
        };
        let mut source = format!("Hook run: {id}\n{text}");
        let mut metadata = fragment.clone();
        if let Some(fields) = metadata.as_object_mut() {
            fields.remove("hookRunId");
            fields.remove("text");
            if !fields.is_empty() {
                source.push_str(&format!("\nMetadata: {metadata:#}"));
            }
        }
        parts.push(source);
    }
    if parts.is_empty() {
        parts.push("No hook context fragments reported".to_owned());
    }
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "fragments"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            parts.push(format!("Metadata: {metadata:#}"));
        }
    }
    let source = parts.join("\n\n");
    let longest = source.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.saturating_add(1).max(3));
    ActivityDocument {
        title: "Hook context".to_owned(),
        markdown: format!("{fence}text\n{source}\n{fence}"),
    }
    .to_snapshot()
    .unwrap_or_else(fallback)
}

fn function_output_snapshot(item: &Value) -> String {
    let Some(name) = item
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
    else {
        return format!("Function output\n{item:#}");
    };
    let tool = item
        .get("namespace")
        .and_then(Value::as_str)
        .filter(|namespace| !namespace.is_empty())
        .map_or_else(
            || name.to_owned(),
            |namespace| format!("{namespace}.{name}"),
        );
    let mut metadata = item.clone();
    let mut heading = "Function output".to_owned();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "name", "namespace", "output"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            heading.push_str(&format!("\nMetadata: {metadata:#}"));
        }
    }
    let mut content = vec![json!({"type":"text","text":heading,"source":item})];
    let plain = match item.get("output") {
        Some(Value::String(text)) => {
            content.push(
                json!({"type":"text","text":if text.is_empty() { "(empty output)" } else { text }}),
            );
            text.clone()
        },
        Some(Value::Array(items)) => {
            content.extend(items.iter().map(function_output_block));
            if items.is_empty() {
                content.push(json!({"type":"text","text":"(empty output)"}));
            }
            format!("{}", Value::Array(items.clone()))
        },
        Some(value) => {
            content.push(json!({"type":"text","text":format!("{value:#}")}));
            format!("{value:#}")
        },
        None => {
            content.push(json!({"type":"text","text":"No output value reported"}));
            "No output value reported".to_owned()
        },
    };
    let plain_text = format!("{tool}\n{heading}\n{plain}");
    ToolOutput {
        tool,
        server: None,
        arguments: None,
        result: None,
        content_items: Some(Value::Array(content)),
        error: None,
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Function output\n{item:#}"))
}

fn function_output_block(block: &Value) -> Value {
    let mut normalized = block.clone();
    let Some(fields) = normalized.as_object_mut() else {
        return normalized;
    };
    match fields.get("type").and_then(Value::as_str) {
        Some("input_text") if fields.get("text").is_some_and(Value::is_string) => {
            fields.insert("type".to_owned(), json!("text"));
        },
        Some("input_image" | "input_audio") => {
            let image = fields["type"] == "input_image";
            let (old, new, kind) = if image {
                ("image_url", "imageUrl", "inputImage")
            } else {
                ("audio_url", "audioUrl", "inputAudio")
            };
            if fields.get(old).is_some_and(Value::is_string) && !fields.contains_key(new) {
                let url = fields.remove(old).expect("validated URL remains present");
                fields.insert(new.to_owned(), url);
                fields.insert("type".to_owned(), json!(kind));
            }
        },
        _ => {},
    }
    normalized
}

fn sub_agent_activity_snapshot(item: &Value) -> String {
    let Some(kind) = item.get("kind").and_then(Value::as_str) else {
        return format!("Agent activity\n{item:#}");
    };
    let mut lines = Vec::new();
    for (field, label) in [
        ("agentPath", "Agent path"),
        ("agentThreadId", "Agent thread"),
    ] {
        if let Some(value) = item.get(field) {
            lines.push(format!(
                "{label}: {}",
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{value:#}"))
            ));
        }
    }
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "kind", "agentPath", "agentThreadId"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            lines.push(format!("Metadata: {metadata:#}"));
        }
    }
    ActivityNotice {
        title: format!("Agent activity · {kind}"),
        message: lines.join("\n"),
        level: NoticeLevel::Info,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Agent activity\n{item:#}"))
}

fn sleep_snapshot(item: &Value) -> String {
    let Some(duration) = item.get("durationMs").and_then(Value::as_u64) else {
        return format!("Wait\n{item:#}");
    };
    let mut plain_text = format!("Wait\nRequested duration: {duration} ms");
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "durationMs"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            plain_text.push_str(&format!("\nMetadata: {metadata:#}"));
        }
    }
    ToolOutput {
        tool: "clock.sleep".to_owned(),
        server: None,
        arguments: Some(json!({"durationMs":duration})),
        result: None,
        content_items: Some(json!([{"type":"text","text":plain_text,"source":item}])),
        error: None,
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Wait\n{item:#}"))
}

fn collab_tool_snapshot(item: &Value) -> String {
    let Some(tool) = item
        .get("tool")
        .and_then(Value::as_str)
        .filter(|tool| !tool.is_empty())
    else {
        return format!("Agent task\n{item:#}");
    };
    let mut parts = vec![format!("Agent task · {tool}")];
    let mut metadata = item.clone();
    for (field, label) in [
        ("status", "Tool status"),
        ("senderThreadId", "Sender"),
        ("receiverThreadIds", "Recipients"),
        ("agentsStates", "Reported agent states"),
        ("model", "Requested model"),
        ("reasoningEffort", "Requested reasoning effort"),
        ("prompt", "Prompt"),
    ] {
        if let Some(value) = item.get(field).filter(|value| !value.is_null()) {
            let text = if field == "agentsStates" {
                collab_agent_states(value)
            } else {
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{value:#}"))
            };
            parts.push(format!("{label}: {text}"));
        }
        if let Some(fields) = metadata.as_object_mut() {
            fields.remove(field);
        }
    }
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "tool"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            parts.push(format!("Metadata: {metadata:#}"));
        }
    }
    let plain_text = parts.join("\n");
    ToolOutput {
        tool: tool.to_owned(),
        server: None,
        arguments: None,
        result: None,
        content_items: Some(json!([{"type":"text","text":plain_text,"source":item}])),
        error: None,
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Agent task\n{item:#}"))
}

fn collab_agent_states(value: &Value) -> String {
    let Some(states) = value.as_object() else {
        return format!("{value:#}");
    };
    if states.is_empty() {
        return "(none reported)".to_owned();
    }
    states
        .iter()
        .map(|(id, state)| {
            let Some(status) = state.get("status").and_then(Value::as_str) else {
                return format!("Agent {id}\n{state:#}");
            };
            let mut text = format!("Agent {id} · {status}");
            let mut metadata = state.clone();
            if let Some(fields) = metadata.as_object_mut() {
                fields.remove("status");
                match fields.get("message") {
                    Some(Value::String(message)) => {
                        if !message.is_empty() {
                            text.push_str(&format!("\n{message}"));
                        }
                        fields.remove("message");
                    },
                    Some(Value::Null) => {
                        fields.remove("message");
                    },
                    _ => {},
                }
                if !fields.is_empty() {
                    text.push_str(&format!("\n{metadata:#}"));
                }
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn image_view_snapshot(item: &Value) -> String {
    let Some(path) = item.get("path").and_then(Value::as_str) else {
        return format!("Image view\n{item:#}");
    };
    let mut plain_text =
        format!("Image view\nPath: {path}\nImage bytes are not included in this event.");
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in ["id", "type", "path"] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            plain_text.push_str(&format!("\nMetadata: {metadata:#}"));
        }
    }
    ToolOutput {
        tool: "view_image".to_owned(),
        server: None,
        arguments: Some(json!({"path":path})),
        result: None,
        content_items: Some(json!([{"type":"text","text":plain_text,"source":item}])),
        error: None,
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Image view\n{item:#}"))
}

fn image_generation_snapshot(item: &Value) -> String {
    let mut details = Vec::new();
    for (field, label) in [
        ("status", "Status"),
        ("revisedPrompt", "Prompt"),
        ("savedPath", "Saved path"),
    ] {
        if let Some(text) = item
            .get(field)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            details.push(format!("{label}: {text}"));
        }
    }
    if let Some(failure) = item.get("failure").filter(|value| !value.is_null()) {
        details.push(format!("Failure: {failure:#}"));
    }
    let mut metadata = item.clone();
    if let Some(fields) = metadata.as_object_mut() {
        for field in [
            "type",
            "id",
            "status",
            "revisedPrompt",
            "savedPath",
            "failure",
            "result",
        ] {
            fields.remove(field);
        }
        if !fields.is_empty() {
            details.push(format!("Metadata: {metadata:#}"));
        }
    }
    let data = item
        .get("result")
        .and_then(Value::as_str)
        .filter(|data| !data.is_empty());
    details.push(if data.is_some() {
        "Generated PNG image (original payload retained)".to_owned()
    } else {
        "No image payload received".to_owned()
    });
    let plain_text = format!("Image generation\n{}", details.join("\n"));
    // Keep the complete provider item for custom renderers; common image blocks
    // carry the PNG payload and never load savedPath from the client filesystem.
    let mut content = vec![json!({"type":"text","text":plain_text,"source":item})];
    if let Some(data) = data {
        content.push(json!({"type":"image","mimeType":"image/png","data":data}));
    }
    ToolOutput {
        tool: "image_generation".to_owned(),
        server: None,
        arguments: None,
        result: None,
        content_items: Some(Value::Array(content)),
        error: item
            .get("failure")
            .filter(|value| !value.is_null())
            .cloned(),
        plain_text,
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("Image generation\n{item:#}"))
}

fn review_snapshot(item: &Value, ended: bool) -> String {
    let title = if ended {
        "Review ended"
    } else {
        "Review started"
    };
    let Some(markdown) = item.get("review").and_then(Value::as_str) else {
        return format!("{title}\n{item:#}");
    };
    ActivityDocument {
        title: title.to_owned(),
        markdown: markdown.to_owned(),
    }
    .to_snapshot()
    .unwrap_or_else(|| format!("{title}\n{item:#}"))
}

fn proposed_plan_snapshot(markdown: &str) -> Result<String, BackendFailure> {
    ActivityDocument {
        title: "Proposed plan".to_owned(),
        markdown: markdown.to_owned(),
    }
    .to_snapshot()
    .ok_or_else(|| protocol::protocol_failure("proposed plan exceeds output profile limit"))
}

fn reasoning_snapshot(summary: String) -> Option<String> {
    ActivitySummary {
        kind: SummaryKind::Reasoning,
        summary,
        tokens_before: None,
    }
    .to_snapshot()
}

fn compaction_snapshot(completed: bool) -> String {
    ActivityNotice {
        title: if completed {
            "Context compacted"
        } else {
            "Compacting context"
        }
        .to_owned(),
        message: if completed {
            "Codex compacted the conversation context."
        } else {
            "Codex is compacting the conversation context."
        }
        .to_owned(),
        level: NoticeLevel::Info,
    }
    .to_snapshot()
    .expect("bounded static compaction notice")
}

fn web_search_snapshot(item: &Value) -> String {
    let mut plain_text = web_search_text(item);
    // The app-server deliberately leaves result entries opaque. Preserve new
    // result kinds and fields without inventing a universal citation schema.
    let result = item
        .get("results")
        .filter(|value| !value.is_null())
        .map(|results| {
            plain_text.push_str(&format!("\nResults:\n{results:#}"));
            json!({"results":results})
        });
    let arguments = ["query", "action"]
        .into_iter()
        .filter_map(|key| item.get(key).map(|value| (key.to_owned(), value.clone())))
        .collect::<Map<_, _>>();
    ToolOutput {
        tool: "webSearch".to_owned(),
        server: None,
        arguments: Some(Value::Object(arguments)),
        result,
        content_items: None,
        error: None,
        plain_text: plain_text.clone(),
    }
    .to_snapshot()
    .unwrap_or(plain_text)
}

fn web_search_text(item: &Value) -> String {
    let action = item.get("action").filter(|value| !value.is_null());
    let action_type = action
        .and_then(|action| action.get("type"))
        .and_then(Value::as_str);
    let mut lines = vec![
        match action_type {
            Some("openPage") => "Open web page",
            Some("findInPage") => "Find in web page",
            Some("search") => "Web search",
            None if action.is_none() => "Web search",
            _ => "Web search · other action",
        }
        .to_owned(),
    ];
    let mut queries = Vec::new();
    if let Some(query) = action
        .and_then(|action| action.get("query"))
        .and_then(Value::as_str)
        .filter(|query| !query.is_empty())
    {
        queries.push(query);
    }
    if let Some(values) = action
        .and_then(|action| action.get("queries"))
        .and_then(Value::as_array)
    {
        for query in values
            .iter()
            .filter_map(Value::as_str)
            .filter(|query| !query.is_empty())
        {
            if !queries.contains(&query) {
                queries.push(query);
            }
        }
    }
    lines.extend(queries.into_iter().map(|query| format!("Query: {query}")));
    for (field, label) in [("url", "URL"), ("pattern", "Find")] {
        if let Some(value) = action
            .and_then(|action| action.get(field))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            lines.push(format!("{label}: {value}"));
        }
    }
    // Codex keeps an item-level query when no usable action detail is available.
    if lines.len() == 1
        && let Some(query) = item
            .get("query")
            .and_then(Value::as_str)
            .filter(|query| !query.is_empty())
    {
        lines.push(format!("Query: {query}"));
    }
    if let Some(action) = action {
        let known_fields: &[&str] = match action_type {
            Some("search") => &["type", "query", "queries"],
            Some("openPage") => &["type", "url"],
            Some("findInPage") => &["type", "url", "pattern"],
            _ => &[],
        };
        let mut extra = action.clone();
        if let Some(fields) = extra.as_object_mut() {
            for field in known_fields {
                let valid = match *field {
                    "queries" => fields.get(*field).is_some_and(|value| {
                        value.is_null()
                            || value
                                .as_array()
                                .is_some_and(|values| values.iter().all(Value::is_string))
                    }),
                    _ => fields
                        .get(*field)
                        .is_some_and(|value| value.is_null() || value.is_string()),
                };
                if valid {
                    fields.remove(*field);
                }
            }
        }
        if !extra.as_object().is_some_and(Map::is_empty) {
            lines.push(to_string_pretty(&extra).expect("JSON is serializable"));
        }
    }
    if lines.len() == 1 {
        lines.push("Details not reported".to_owned());
    }
    lines.join("\n")
}

fn checked_command_snapshot(item: &Value) -> Result<String, BackendFailure> {
    command_snapshot(item)
        .ok_or_else(|| protocol::protocol_failure("command output exceeds output profile limit"))
}

fn command_snapshot(item: &Value) -> Option<String> {
    let mut arguments = Map::new();
    let mut result = Map::new();
    for field in ["command", "cwd"] {
        if let Some(value) = item.get(field) {
            arguments.insert(field.to_owned(), value.clone());
        }
    }
    for field in ["exitCode", "durationMs", "status"] {
        if let Some(value) = item.get(field) {
            result.insert(field.to_owned(), value.clone());
        }
    }
    if let Some(output) = item
        .get("aggregatedOutput")
        .filter(|value| !value.is_null())
    {
        if let Some(text) = output.as_str() {
            result.insert(
                "content".to_owned(),
                json!([{"type": "text", "text": text}]),
            );
        } else {
            result.insert("aggregatedOutput".to_owned(), output.clone());
        }
    }
    ToolOutput {
        tool: "commandExecution".to_owned(),
        server: None,
        arguments: (!arguments.is_empty()).then_some(Value::Object(arguments)),
        result: (!result.is_empty()).then_some(Value::Object(result)),
        content_items: None,
        error: None,
        plain_text: command_plain_text(item).unwrap_or_else(|| "Command execution".to_owned()),
    }
    .to_snapshot()
}

fn command_plain_text(item: &Value) -> Option<String> {
    let command = item.get("command").and_then(Value::as_str);
    let output = item.get("aggregatedOutput").and_then(Value::as_str);
    let mut lines = Vec::new();
    if let Some(command) = command {
        lines.push(format!("$ {command}"));
    }
    if let Some(cwd) = item.get("cwd").and_then(Value::as_str) {
        lines.push(format!("Directory: {cwd}"));
    }
    if let Some(output) = output.filter(|output| !output.is_empty()) {
        lines.push(output.to_owned());
    }
    let mut result = Vec::new();
    if let Some(code) = item.get("exitCode").and_then(Value::as_i64) {
        result.push(format!("Exit: {code}"));
    }
    if let Some(duration) = item.get("durationMs").and_then(Value::as_u64) {
        result.push(format!("Duration: {duration} ms"));
    }
    if !result.is_empty() {
        lines.push(result.join(" · "));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn file_change_snapshot(item: &Value) -> Option<String> {
    let changes = item.get("changes")?.as_array()?;
    let lines = changes
        .iter()
        .filter_map(|change| {
            let path = change.get("path")?.as_str()?;
            let kind = change.get("kind");
            let name = kind
                .and_then(|kind| kind.as_str().or_else(|| kind.get("type")?.as_str()))
                .unwrap_or("update");
            let mut text = format!("{name}: {path}");
            if let Some(destination) = kind
                .and_then(|kind| kind.get("move_path"))
                .and_then(Value::as_str)
            {
                text.push_str(&format!(" -> {destination}"));
            }
            if let Some(diff) = change
                .get("diff")
                .and_then(Value::as_str)
                .filter(|diff| !diff.is_empty())
            {
                text.push('\n');
                match name {
                    "add" | "delete" => {
                        // App-server sends whole file content for add/delete;
                        // only update carries a unified diff.
                        let prefix = if name == "add" { '+' } else { '-' };
                        for line in diff.split_inclusive('\n') {
                            text.push(prefix);
                            text.push_str(line);
                        }
                        if !diff.ends_with('\n') {
                            text.push_str("\n\\ No newline at end of file");
                        }
                    },
                    _ => text.push_str(diff),
                }
            }
            Some(text)
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn tool_snapshot(item: &Value) -> Option<String> {
    let tool = item.get("tool")?.as_str()?;
    let mut sections = vec![match item.get("server").and_then(Value::as_str) {
        Some(server) => format!("{server}.{tool}"),
        None => tool.to_owned(),
    }];
    for (field, label) in [
        ("arguments", "Arguments"),
        ("result", "Result"),
        ("contentItems", "Result"),
        ("error", "Error"),
    ] {
        if let Some(value) = item.get(field).filter(|value| !value.is_null()) {
            let text = match field {
                "result" => mcp_result_text(value),
                "contentItems" => tool_content_text(value),
                _ => None,
            }
            .unwrap_or_else(|| json_or_text(value));
            sections.push(format!("{label}:\n{text}"));
        }
    }
    let output = ToolOutput {
        tool: tool.to_owned(),
        server: item
            .get("server")
            .and_then(Value::as_str)
            .map(str::to_owned),
        arguments: item
            .get("arguments")
            .filter(|value| !value.is_null())
            .cloned(),
        result: item.get("result").filter(|value| !value.is_null()).cloned(),
        content_items: item
            .get("contentItems")
            .filter(|value| !value.is_null())
            .cloned(),
        error: item.get("error").filter(|value| !value.is_null()).cloned(),
        plain_text: sections.join("\n"),
    };
    Some(output.to_snapshot().unwrap_or(output.plain_text))
}

fn json_or_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| to_string_pretty(value).expect("JSON values are serializable"))
}

fn mcp_result_text(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let mut text = tool_content_text(object.get("content")?)?;
    // Keep structured output and extension metadata independently of human text.
    for (key, value) in object.iter().filter(|(key, _)| *key != "content") {
        text.push_str(&format!("\n{key}:\n{}", json_or_text(value)));
    }
    Some(text)
}

fn tool_content_text(value: &Value) -> Option<String> {
    let blocks = value.as_array()?;
    if blocks.is_empty() {
        return Some("(empty content)".to_owned());
    }
    Some(
        blocks
            .iter()
            .map(|block| tool_content_block(block).unwrap_or_else(|| json_or_text(block)))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

fn tool_content_block(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let kind = object.get("type")?.as_str()?;
    let (mut text, consumed): (String, &[&str]) = match kind {
        "text" | "inputText" => (object.get("text")?.as_str()?.to_owned(), &["type", "text"]),
        "image" | "audio" => {
            let data = object.get("data")?.as_str()?;
            let mime = object.get("mimeType")?.as_str()?;
            let label = if kind == "image" { "Image" } else { "Audio" };
            (
                format!(
                    "{label} · {mime}\nEmbedded data: {} encoded bytes",
                    data.len()
                ),
                &["type", "data", "mimeType"],
            )
        },
        "inputImage" | "inputAudio" => {
            let field = if kind == "inputImage" {
                "imageUrl"
            } else {
                "audioUrl"
            };
            let url = object.get(field)?.as_str()?;
            let label = if kind == "inputImage" {
                "Image"
            } else {
                "Audio"
            };
            let text = if let Some((mime, data)) = url
                .strip_prefix("data:")
                .and_then(|url| url.split_once(";base64,"))
            {
                format!(
                    "{label} · {mime}\nEmbedded data: {} encoded bytes",
                    data.len()
                )
            } else {
                format!("{label} URL: {url}")
            };
            (
                text,
                if kind == "inputImage" {
                    &["type", "imageUrl"]
                } else {
                    &["type", "audioUrl"]
                },
            )
        },
        "resource_link" => {
            let uri = object.get("uri")?.as_str()?;
            let name = object
                .get("title")
                .or_else(|| object.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("Resource");
            (format!("Resource · {name}\nURI: {uri}"), &["type", "uri"])
        },
        "resource" => {
            let resource = object.get("resource")?.as_object()?;
            let uri = resource.get("uri")?.as_str()?;
            // Text resources are readable; binary and unfamiliar shapes retain their
            // complete JSON until a typed media handoff is available.
            let body = resource.get("text")?.as_str()?;
            let mut text = format!("Resource\nURI: {uri}\n{body}");
            for (key, value) in resource
                .iter()
                .filter(|(key, _)| !["uri", "text"].contains(&key.as_str()))
            {
                text.push_str(&format!("\n{key}: {}", json_or_text(value)));
            }
            (text, &["type", "resource"])
        },
        _ => return None,
    };
    for (key, value) in object
        .iter()
        .filter(|(key, _)| !consumed.contains(&key.as_str()))
    {
        text.push_str(&format!("\n{key}: {}", json_or_text(value)));
    }
    Some(text)
}

// Mirrors Codex e1eb98461cd4 tui/approval_events.rs::default_available_decisions
// and bottom_pane/approval_overlay.rs::patch_options. Explicit server lists always win.
fn approval_decisions(method: &str, params: &Value) -> Result<(Vec<Value>, bool), BackendFailure> {
    match params.get("availableDecisions") {
        Some(Value::Array(choices)) => return Ok((choices.clone(), true)),
        None | Some(Value::Null) => {},
        Some(_) => {
            return Err(protocol::protocol_failure(
                "approval availableDecisions must be an array or null",
            ));
        },
    }
    if method == "item/fileChange/requestApproval" {
        return Ok((
            vec![json!("accept"), json!("acceptForSession"), json!("cancel")],
            false,
        ));
    }
    let present = |key| params.get(key).filter(|value| !value.is_null());
    let mut decisions = vec![json!("accept")];
    if let Some(network) = present("networkApprovalContext") {
        if network.get("host").and_then(Value::as_str).is_none()
            || !matches!(
                network.get("protocol").and_then(Value::as_str),
                Some("http" | "https" | "socks5Tcp" | "socks5Udp")
            )
        {
            return Err(protocol::protocol_failure(
                "invalid network approval context",
            ));
        }
        decisions.push(json!("acceptForSession"));
        if let Some(proposals) = present("proposedNetworkPolicyAmendments") {
            let proposals = proposals
                .as_array()
                .ok_or_else(|| protocol::protocol_failure("invalid proposed network policies"))?;
            let mut first_allow = None;
            for proposal in proposals {
                let decision =
                    json!({"applyNetworkPolicyAmendment":{"network_policy_amendment":proposal}});
                if approval_choice(&decision, true).is_none() {
                    return Err(protocol::protocol_failure(
                        "invalid proposed network policy",
                    ));
                }
                if first_allow.is_none()
                    && proposal.get("action").and_then(Value::as_str) == Some("allow")
                {
                    first_allow = Some(decision);
                }
            }
            decisions.extend(first_allow);
        }
    } else if let Some(permissions) = present("additionalPermissions") {
        if !permissions.is_object() {
            return Err(protocol::protocol_failure(
                "invalid additional approval permissions",
            ));
        }
    } else if let Some(prefix) = present("proposedExecpolicyAmendment") {
        let decision = json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":prefix}});
        if approval_choice(&decision, true).is_none() {
            return Err(protocol::protocol_failure(
                "invalid proposed command policy",
            ));
        }
        decisions.push(decision);
    }
    decisions.push(json!("cancel"));
    Ok((decisions, false))
}

pub(super) fn approval_choice(value: &Value, command: bool) -> Option<ApprovalChoice> {
    if !command && !value.is_string() {
        return None;
    }
    let (label, description) = match value.as_str() {
        Some("accept") => (
            "Approve request",
            "Use the scope described in this request.".to_owned(),
        ),
        Some("acceptForSession") => (
            "Approve for session",
            "Future matching prompts in this session may run without asking again.".to_owned(),
        ),
        Some("decline") => (
            "Decline",
            "Do not run this action; continue the turn.".to_owned(),
        ),
        Some("cancel") => (
            "Decline and stop",
            "Do not run this action; interrupt the turn.".to_owned(),
        ),
        Some(_) => return None,
        None => {
            let fields = value.as_object()?;
            if fields.len() != 1 {
                return None;
            }
            if let Some(amendment) = fields.get("acceptWithExecpolicyAmendment") {
                let fields = amendment.as_object()?;
                if fields.len() != 1 {
                    return None;
                }
                let prefix = fields.get("execpolicy_amendment")?.as_array()?;
                if !prefix.iter().all(Value::is_string) {
                    return None;
                }
                (
                    "Approve + save rule",
                    format!(
                        "Persistent rule: future matching commands may run without prompting. Prefix: {}",
                        serde_json::to_string(prefix).ok()?
                    ),
                )
            } else {
                let fields = fields.get("applyNetworkPolicyAmendment")?.as_object()?;
                if fields.len() != 1 {
                    return None;
                }
                let rule = fields.get("network_policy_amendment")?.as_object()?;
                if rule.len() != 2 {
                    return None;
                }
                let host = rule.get("host")?.as_str()?;
                match rule.get("action")?.as_str()? {
                    "allow" => (
                        "Always allow host",
                        format!("Persistent network allow rule for host: {host}"),
                    ),
                    "deny" => (
                        "Always deny host",
                        format!("Persistent network deny rule for host: {host}"),
                    ),
                    _ => return None,
                }
            }
        },
    };
    Some(ApprovalChoice {
        label: label.to_owned(),
        description,
        enabled: true,
    })
}

fn approval_summary(method: &str, params: &Value) -> Result<String, BackendFailure> {
    let mut output = ApprovalText(Vec::new());
    let render = |output: &mut ApprovalText| -> Result<(), io::Error> {
        output.write_all(if method == "item/fileChange/requestApproval" {
            b"File change approval"
        } else {
            b"Command approval"
        })?;
        for (key, label) in [
            ("kind", "Action kind"),
            ("command", "Command"),
            ("cwd", "Working directory"),
            ("environmentId", "Environment"),
            (
                "grantRoot",
                if method == "item/fileChange/requestApproval" {
                    "Requested write root (session scope)"
                } else {
                    "Reported grantRoot"
                },
            ),
            ("reason", "Reason"),
            ("networkApprovalContext", "Network target"),
            ("additionalPermissions", "Additional permissions"),
            ("commandActions", "Reported command actions"),
            ("proposedExecpolicyAmendment", "Proposed command policy"),
            (
                "proposedNetworkPolicyAmendments",
                "Proposed network policies",
            ),
            ("availableDecisions", "Offered decisions"),
        ] {
            let Some(value) = params.get(key).filter(|value| !value.is_null()) else {
                continue;
            };
            output.write_all(b"\n")?;
            output.write_all(label.as_bytes())?;
            output.write_all(b": ")?;
            match value {
                Value::String(text) => output.write_all(text.as_bytes())?,
                value => serde_json::to_writer(&mut *output, value).map_err(io::Error::other)?,
            }
        }
        if let Some(fields) = params.as_object() {
            for (key, value) in fields.iter().filter(|(key, _)| {
                ![
                    "threadId",
                    "turnId",
                    "itemId",
                    "approvalId",
                    "startedAtMs",
                    "kind",
                    "command",
                    "cwd",
                    "environmentId",
                    "grantRoot",
                    "reason",
                    "networkApprovalContext",
                    "additionalPermissions",
                    "commandActions",
                    "proposedExecpolicyAmendment",
                    "proposedNetworkPolicyAmendments",
                    "availableDecisions",
                ]
                .contains(&key.as_str())
            }) {
                output.write_all(b"\nAdditional field ")?;
                serde_json::to_writer(&mut *output, key).map_err(io::Error::other)?;
                output.write_all(b": ")?;
                serde_json::to_writer(&mut *output, value).map_err(io::Error::other)?;
            }
        }
        Ok(())
    };
    render(&mut output)
        .map_err(|_| protocol::protocol_failure("approval context exceeds the display limit"))?;
    Ok(String::from_utf8(output.0).expect("approval fields and JSON are UTF-8"))
}

struct ApprovalText(Vec<u8>);

impl Write for ApprovalText {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|size| size > ToolOutput::MAX_SNAPSHOT_BYTES)
        {
            return Err(io::Error::other("approval display limit exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl InputQuestions {
    fn parse(params: &Value) -> Result<Self, BackendFailure> {
        let questions = params
            .get("questions")
            .and_then(Value::as_array)
            .filter(|questions| !questions.is_empty())
            .ok_or_else(|| protocol::protocol_failure("user-input request has no questions"))?;
        let mut ids = HashSet::new();
        let mut parsed = Vec::with_capacity(questions.len());
        for question in questions {
            if question
                .get("isSecret")
                .is_some_and(|secret| secret != &Value::Bool(false))
            {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "secret input requires a secure editor; yo will not echo it into the transcript",
                ));
            }
            let other = match question.get("isOther") {
                None => false,
                Some(Value::Bool(value)) => *value,
                Some(_) => {
                    return Err(protocol::protocol_failure(
                        "question isOther must be boolean",
                    ));
                },
            };
            let id = protocol::string_at(question, &["id"])?;
            if id.is_empty() || !ids.insert(id) {
                return Err(protocol::protocol_failure(
                    "user-input question IDs must be nonempty and unique",
                ));
            }
            let header = protocol::string_at(question, &["header"])?;
            let body = protocol::string_at(question, &["question"])?;
            let mut prompt = format!("{header}\n\n{body}");
            let mut options = Vec::new();
            let mut choices = Vec::new();
            if let Some(values) = question.get("options").filter(|value| !value.is_null()) {
                let values = values.as_array().ok_or_else(|| {
                    protocol::protocol_failure("question options must be an array")
                })?;
                for value in values {
                    let label = protocol::string_at(value, &["label"])?;
                    let description = protocol::string_at(value, &["description"])?;
                    options.push(label.to_owned());
                    choices.push(QuestionChoice {
                        label: label.to_owned(),
                        description: description.to_owned(),
                    });
                    prompt.push_str(&format!("\n{}. {label} — {description}", options.len()));
                }
            }
            if other && !options.is_empty() {
                let label = "None of the above";
                let description = "Choose an answer outside this list.";
                options.push(label.to_owned());
                choices.push(QuestionChoice {
                    label: label.to_owned(),
                    description: description.to_owned(),
                });
                prompt.push_str(&format!("\n{}. {label} — {description}", options.len()));
            }
            parsed.push(InputQuestion {
                id: id.to_owned(),
                prompt,
                question: body.to_owned(),
                options,
                choices,
            });
        }
        Ok(Self {
            questions: parsed,
            current: 0,
            answers: Map::new(),
            drafts: HashMap::new(),
        })
    }

    pub(super) fn incomplete_notice(&self, reason: &str) -> String {
        let recorded = self.answers.len();
        let remaining = self.questions.len().saturating_sub(recorded);
        let base = format!(
            "{recorded}/{} answers recorded · {remaining} unanswered\nSubmission incomplete.\n{reason}",
            self.questions.len()
        );
        let fallback = || {
            ActivityNotice {
                level: NoticeLevel::Warning,
                title: "Interview incomplete".to_owned(),
                message: format!(
                    "{base}\nQuestion list omitted: summary exceeds the output limit."
                ),
            }
            .to_snapshot()
            .expect("bounded summary counts and static reason")
        };
        let mut message = base.clone();
        for (index, question) in self.questions.iter().enumerate() {
            let state = if self.answers.contains_key(&question.id) {
                "Recorded"
            } else {
                "Unanswered"
            };
            let prefix = format!("\n{}. {state}: ", index + 1);
            if message
                .len()
                .checked_add(prefix.len())
                .and_then(|size| size.checked_add(question.question.len()))
                .is_none_or(|size| size > ToolOutput::MAX_SNAPSHOT_BYTES)
            {
                return fallback();
            }
            message.push_str(&prefix);
            message.push_str(&question.question);
        }
        ActivityNotice {
            level: NoticeLevel::Warning,
            title: "Interview incomplete".to_owned(),
            message,
        }
        .to_snapshot()
        .unwrap_or_else(fallback)
    }

    pub(super) fn receipt(&self, answer: &str, notes: Option<&str>) -> String {
        let progress = format!(
            "Question {} of {}\n",
            self.current + 1,
            self.questions.len()
        );
        let delivery = if self.current + 1 == self.questions.len() {
            "All question responses sent."
        } else {
            "Recorded; waiting for the remaining questions."
        };
        let question = &self.questions[self.current].question;
        let parts = [
            progress.as_str(),
            question,
            "\n\nAnswer: ",
            answer,
            if notes.is_some() { "\nNote: " } else { "" },
            notes.unwrap_or(""),
            "\n\n",
            delivery,
        ];
        let size = parts
            .iter()
            .try_fold(0_usize, |size, part| size.checked_add(part.len()));
        if size.is_none_or(|size| size > ToolOutput::MAX_SNAPSHOT_BYTES) {
            return format!(
                "{progress}{delivery}\nAnswer display omitted: receipt exceeds the output limit. The original response remains in the session journal."
            );
        }
        let mut text = String::with_capacity(size.expect("bounded receipt size"));
        for part in parts {
            text.push_str(part);
        }
        text
    }

    pub(super) fn question_profile(&self, index: usize) -> ActivityQuestion {
        let question = &self.questions[index];
        let hint = if question.options.is_empty() {
            "Enter your answer."
        } else {
            "Enter a number or write your answer."
        };
        let submission = if self.questions.len() == 1 {
            "Submitting this answer sends your response.".to_owned()
        } else if index + 1 == self.questions.len() {
            format!(
                "Submitting this answer sends all {} answers.",
                self.questions.len()
            )
        } else {
            "This answer is recorded locally. All answers are sent after the final question."
                .to_owned()
        };
        let plain_text = format!(
            "Question {} of {} · {}\n\n{hint}\n{submission}\nEsc interrupts the turn.",
            index + 1,
            self.questions.len(),
            question.prompt
        );
        ActivityQuestion {
            allow_notes: true,
            previous_question: index > 0,
            draft: self.drafts.get(&question.id).map(|(_, text)| text.clone()),
            draft_choice: self
                .drafts
                .get(&question.id)
                .and_then(|(choice, _)| *choice),
            plain_text,
            choices: question.choices.clone(),
        }
    }

    pub(super) fn prompt(&self) -> String {
        let mut profile = self.question_profile(self.current);
        profile.previous_question = self.current > 0
            && self
                .question_profile(self.current - 1)
                .to_snapshot()
                .is_some();
        profile.to_snapshot().unwrap_or(profile.plain_text)
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
