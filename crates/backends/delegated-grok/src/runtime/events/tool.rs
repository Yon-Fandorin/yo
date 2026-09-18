use std::time::Duration;

use serde_json::{Value, json};
use similar::TextDiff;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityUpdate, BackendEvent, BackendFailure, Failure,
    ToolOutput,
};

use super::super::state::{
    Backend, ToolBinding, ToolIdentity, format_tool_summary, identifier_at, non_empty_text,
    raw_input_summary,
};
use crate::{protocol, transport::JsonPeer};

impl<P: JsonPeer> Backend<P> {
    pub(super) fn tool_call(
        &mut self,
        update: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let turn = self.active_turn()?;
        let tool_id = identifier_at(update, "toolCallId")?.to_owned();
        if self
            .input_tool_turns
            .get(&tool_id)
            .is_some_and(|question_turn| *question_turn != turn)
        {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP tool call `{tool_id}` belongs to an earlier user-question Turn"
            )));
        }
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

    pub(super) fn queue_tool_progress(
        &mut self,
        tool_id: &str,
        update: &Value,
    ) -> Result<(), BackendFailure> {
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

    pub(super) fn tool_call_update(
        &mut self,
        update: &Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
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
