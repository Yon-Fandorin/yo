use serde_json::{Value, json};
use yo_core::{
    ActivityApproval, ActivityKind, ActivityRequestRef, ActivityUpdate, ApprovalChoice,
    BackendEvent, BackendFailure,
};

use super::super::state::{
    ApprovalBinding, Backend, MAX_ACP_IDENTIFIER_BYTES, format_tool_summary, identifier_at,
    non_empty_text, raw_input_summary, wire_key,
};
use crate::{protocol, transport::JsonPeer};

pub(super) fn map_server_request<P: JsonPeer>(
    backend: &mut Backend<P>,
    wire_id: Value,
    method: &str,
    params: Value,
) -> Result<Option<BackendEvent>, BackendFailure> {
    backend.map_permission_request(wire_id, method, params)
}

impl<P: JsonPeer> Backend<P> {
    fn map_permission_request(
        &mut self,
        wire_id: Value,
        method: &str,
        params: Value,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        debug_assert_eq!(method, "session/request_permission");
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
        if self.wire_approvals.contains_key(&wire_key) || self.wire_inputs.contains_key(&wire_key) {
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
