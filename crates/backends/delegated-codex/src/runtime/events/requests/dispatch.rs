//! Codex 서버 요청의 결합, 해석, Activity 수명 주기를 담당합니다.

use std::sync::Arc;

use serde_json::Value;
use yo_backend::transport::JsonMessagePeer;
use yo_core::{
    ActivityApproval, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef,
    ActivityUpdate, ApprovalChoice, BackendEvent, BackendFailure, BackendFailureKind,
    interview::{Capture, InterviewOption, InterviewQuestion},
};

use super::{
    super::super::state::{Backend, InputQuestions, RequestBinding, RequestKind},
    approval::{approval_choice, approval_decisions},
    presentation::approval_summary,
};
use crate::protocol;

pub(in crate::runtime::events) fn wire_key(value: &Value) -> Result<String, BackendFailure> {
    serde_json::to_string(value)
        .map_err(|error| protocol::protocol_failure(format!("invalid Codex request id: {error}")))
}

impl<P: JsonMessagePeer> Backend<P> {
    pub(in crate::runtime::events) fn map_server_request(
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
                | "item/tool/call"
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
        let is_approval = !matches!(method, "item/tool/requestUserInput" | "item/tool/call");
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
            let parsed = if method == "item/tool/call" {
                super::super::super::secret_probe::parse_request(self, &params)
            } else {
                InputQuestions::parse(&params)
            };
            match parsed {
                Ok(questions) => RequestKind::Input(questions),
                Err(error) => {
                    let message = if method == "item/tool/call" {
                        "dynamic tool request is invalid or unavailable"
                    } else {
                        "user-input questions are invalid or require unsupported secret input"
                    };
                    self.client.reject(wire_id, -32602, message)?;
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
                let choices = offered
                    .iter()
                    .map(|value| {
                        approval_choice(value, *command).unwrap_or(ApprovalChoice {
                            label: "Unsupported decision".to_owned(),
                            description: "Review the reported decision in request history; this adapter cannot submit it.".to_owned(),
                            enabled: false,
                        })
                    })
                    .collect();
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
            let capture = Capture::batch(
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
                        allow_notes: !q.is_secret,
                        is_secret: q.is_secret,
                    })
                    .collect(),
            );
            if questions.has_secret() {
                let capture = match capture {
                    Ok(capture) => capture,
                    Err(_) => {
                        self.client.reject(
                            wire_id.clone(),
                            -32602,
                            "secret question capture is invalid or exceeds the display limit",
                        )?;
                        return Err(protocol::protocol_failure(
                            "secret question capture is invalid or exceeds the display limit",
                        ));
                    },
                };
                questions.capture = Some(Arc::new(capture));
                if questions
                    .questions
                    .iter()
                    .enumerate()
                    .filter(|(_, question)| question.is_secret)
                    .any(|(index, _)| questions.question_profile(index).to_snapshot().is_none())
                {
                    self.client.reject(
                        wire_id.clone(),
                        -32602,
                        "secret question presentation is invalid or exceeds the display limit",
                    )?;
                    return Err(protocol::protocol_failure(
                        "secret question presentation is invalid or exceeds the display limit",
                    ));
                }
            } else {
                questions.capture = capture.ok().map(Arc::new);
            }
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

    pub(in crate::runtime::events) fn link_file_approvals(
        &mut self,
        item_id: &str,
        activity: ActivityRef,
    ) {
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
            // 나중에 연결될 related activity ID의 최댓값 인코딩까지 admission에서 예약했습니다.
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

    pub(in crate::runtime::events) fn server_request_resolved(
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

    pub(in crate::runtime::events) fn interview_summary_events(
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
