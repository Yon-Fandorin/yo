use serde_json::Value;
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityUpdate, BackendEvent, BackendFailure, Failure,
};

use super::super::{
    super::{
        secret_probe::TOOL_NAME,
        state::{Backend, DynamicToolCall, ItemBinding},
    },
    snapshots::{
        activity_kind, checked_command_snapshot, command_plain_text, compaction_snapshot,
        file_change_snapshot, item_text_snapshot, proposed_plan_snapshot,
    },
};
use crate::protocol;

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn item_started(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
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
        let dynamic_tool_call = (item_type == "dynamicToolCall"
            && params.pointer("/item/tool").and_then(Value::as_str) == Some(TOOL_NAME))
        .then(|| {
            Some(DynamicToolCall {
                tool: params.pointer("/item/tool")?.as_str()?.to_owned(),
                arguments: params
                    .pointer("/item/arguments")?
                    .as_object()?
                    .clone()
                    .into(),
            })
        })
        .flatten();
        let activity = self.next_activity(turn)?;
        if self
            .items
            .insert(
                item_id.clone(),
                ItemBinding {
                    activity,
                    dynamic_tool_call,
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

    pub(super) fn item_completed(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
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

    pub(super) fn item_delta(
        &mut self,
        params: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
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
}
