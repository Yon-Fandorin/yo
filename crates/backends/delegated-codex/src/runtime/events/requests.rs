use std::{
    io::{self, Write},
    sync::Arc,
};

use serde_json::{Value, json};
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityApproval, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef,
    ActivityUpdate, ApprovalChoice, BackendEvent, BackendFailure, BackendFailureKind, ToolOutput,
    interview::{Capture, InterviewOption, InterviewQuestion},
};

use super::super::state::{Backend, InputQuestions, RequestBinding, RequestKind};
use crate::protocol;

pub(super) fn wire_key(value: &Value) -> Result<String, BackendFailure> {
    serde_json::to_string(value)
        .map_err(|error| protocol::protocol_failure(format!("invalid Codex request id: {error}")))
}

impl<P: JsonMessagePeer> Backend<P> {
    pub(super) fn map_server_request(
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
        let mut kind = if !is_approval {
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
        if let RequestKind::Input(questions) = &mut kind {
            questions.capture = Capture::batch(
                request,
                questions
                    .questions
                    .iter()
                    .map(|q| InterviewQuestion {
                        id: q.id.clone(),
                        prompt: q.prompt.clone(),
                        question: q.question.clone(),
                        options: q
                            .choices
                            .iter()
                            .enumerate()
                            .map(|(index, choice)| InterviewOption {
                                id: (index + 1).to_string(),
                                label: choice.label.clone(),
                                description: choice.description.clone(),
                            })
                            .collect(),
                        allow_free_text: true,
                        allow_notes: true,
                        is_secret: false,
                    })
                    .collect(),
            )
            .ok()
            .map(Arc::new);
        }
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

    pub(super) fn link_file_approvals(&mut self, item_id: &str, activity: ActivityRef) {
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

    pub(super) fn server_request_resolved(
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

    pub(super) fn interview_summary_events(
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
