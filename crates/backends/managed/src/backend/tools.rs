//! Serial tool approval, execution, admitted output, and replay handoff.

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde::Deserialize;
use serde_json::{Value, from_str, json};
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityUpdate,
    ApprovalDecision, BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind,
    Failure, FilePublicationEvidence, FilePublicationEvidenceState,
    FilePublicationEvidenceUnavailableReason, ModelReplayItem, ToolApprovalBinding,
    ToolApprovalRequirement, ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionPreparation,
    ToolExecutionRequest, ToolExecutionResult, ToolOutput, ToolSemanticAdmission,
    ToolValidationFailure, ValidatedToolCall,
};

use super::{
    ActiveTool, NativeModelBackend, PendingCall, TOOL_TRUNCATION_MARKER, TurnState, failure,
    map_tool_cleanup,
};

impl NativeModelBackend {
    pub(super) fn advance_tool_queue(
        &mut self,
        state: &mut TurnState,
    ) -> Result<(), BackendFailure> {
        let Some((_, mut pending)) = state.pending_calls.pop_first() else {
            state.start_next_round = true;
            return Ok(());
        };
        let execution_request = self.tool_execution_request(state, &pending.call);
        let (plan, needs_approval) = match self.tool_host.prepare(&execution_request) {
            ToolExecutionPreparation::Automatic(plan) => (plan, false),
            ToolExecutionPreparation::ApprovalRequired(plan) => (plan, true),
            ToolExecutionPreparation::Unavailable(reason) => {
                self.finish_tool_without_execution(
                    state,
                    pending.call,
                    ToolExecutionOutcome::Failed,
                    json!({"error": "tool unavailable", "code": reason.code()}).to_string(),
                )?;
                return Ok(());
            },
        };
        pending.plan = Some(plan);
        if needs_approval
            || pending.call.definition().approval() == ToolApprovalRequirement::Required
        {
            let activity = self.next_activity(state.turn)?;
            let request = ActivityRequestRef::new(activity, self.next_request()?);
            let binding =
                ToolApprovalBinding::new(state.turn, &pending.call, self.tool_host.identity())
                    .with_plan(*pending.plan.as_ref().expect("prepared plan").identity());
            // 실행 원본 대신 이미 의미 보존 정책을 통과한 replay 인자만 표시한다.
            let arguments = state.delta.iter().rev().find_map(|item| match item {
                ModelReplayItem::FunctionCall {
                    call_id, arguments, ..
                } if call_id == pending.call.call_id() => Some(arguments.as_str()),
                _ => None,
            });
            let approval_text = format!(
                "Tool: {}\nScope: this tool call only\nEffect: {:?}\nExecution host: {}\n\nArguments (recorded view):\n{}\n\nCall: {}\nTool ID: {}\nArgument digest: {}",
                pending.call.definition().wire_name(),
                binding.effect(),
                binding.execution_host(),
                arguments.unwrap_or("(not available)"),
                pending.call.call_id(),
                pending.call.definition().id().as_str(),
                binding.argument_digest_hex(),
            );
            let scope = pending
                .plan
                .as_ref()
                .expect("prepared plan")
                .approval_scope();
            let approval_text = if scope.is_empty() {
                approval_text
            } else {
                format!("{approval_text}\n\nRequested access:\n{scope}")
            };
            pending.approval = Some(binding);
            self.queue_activity_text(
                activity,
                ActivityKind::ApprovalRequest {
                    request_id: request.request_id(),
                },
                approval_text,
                None,
            );
            state.awaiting_approval = Some((request, pending));
            return Ok(());
        }
        state.ready_tool = Some(pending);
        Ok(())
    }

    pub(super) fn fail_tool_admission(
        &mut self,
        activity: ActivityRef,
        call_id: String,
        name: String,
        message: &str,
    ) {
        self.events.push_back(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(
                json!({
                    "call_id": call_id,
                    "name": name,
                    "validation_failure": {
                        "code": ToolValidationFailure::SemanticAdmission.code(),
                        "message": message,
                    },
                })
                .to_string(),
            ),
        });
        self.events.push_back(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Failed(tool_validation_failure(
                ToolValidationFailure::SemanticAdmission,
                message,
            )),
        });
    }

    pub(super) fn start_tool_execution(
        &mut self,
        state: &mut TurnState,
        pending: PendingCall,
        activity: ActivityRef,
    ) -> Result<(), BackendFailure> {
        let PendingCall { call, plan, .. } = pending;
        let plan = plan.ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "tool execution has no prepared plan",
            )
        })?;
        let request = self.tool_execution_request(state, &call);
        // 시작이 실패해도 효과의 불확실성은 남으므로 호출 전에 시도 사실을 기록합니다.
        state.open_group_effect_attempted = true;
        match self.tool_host.start_prepared(request, plan) {
            Ok(execution) => {
                state.active_tool = Some(ActiveTool {
                    call,
                    activity,
                    execution,
                });
                if let Some(snapshot) = state
                    .active_tool
                    .as_ref()
                    .and_then(|active| running_tool_output_snapshot(state, active))
                {
                    // 실행 시작이 확인된 뒤 원래 ToolResult Activity에 typed snapshot을 덧붙인다.
                    self.events.push_back(BackendEvent::ActivityUpdated {
                        activity,
                        update: ActivityUpdate::TextSnapshot(snapshot),
                    });
                }
            },
            Err(_) => self.finish_tool(
                state,
                call,
                activity,
                ToolExecutionResult::new(
                    ToolExecutionOutcome::Failed,
                    "tool execution failed",
                    false,
                ),
            )?,
        }
        Ok(())
    }

    fn tool_execution_request(
        &self,
        state: &TurnState,
        call: &ValidatedToolCall,
    ) -> ToolExecutionRequest {
        ToolExecutionRequest {
            turn: state.turn,
            call: call.clone(),
            maximum_output_bytes: self.config.maximum_tool_output_bytes,
            maximum_retained_output_bytes: self.config.maximum_retained_tool_output_bytes,
            absolute_execution_timeout: self.config.absolute_tool_execution_timeout,
        }
    }

    pub(super) fn poll_tool(&mut self) -> Result<(), BackendFailure> {
        let mut state = self.turn.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "tool execution has no active Turn",
            )
        })?;
        if let Err(error) = self.poll_tool_state(&mut state) {
            self.fail_or_exhaust_turn(&mut state, error);
        } else if self.turn.is_none()
            && !self.events.iter().any(|event| {
                matches!(
                    event,
                    BackendEvent::TurnFinished { .. } | BackendEvent::ResumableTurnFinished { .. }
                )
            })
        {
            self.turn = Some(state);
        }
        Ok(())
    }

    fn poll_tool_state(&mut self, state: &mut TurnState) -> Result<(), BackendFailure> {
        let mut active = state.active_tool.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "tool poll has no active execution",
            )
        })?;
        let poll = match active.execution.poll() {
            Ok(poll) => poll,
            Err(error) => {
                state.active_tool = Some(active);
                return Err(map_tool_turn(error));
            },
        };
        match poll {
            ToolExecutionPoll::Pending => {
                let result = self.publish_tool_progress(state, &mut active);
                state.active_tool = Some(active);
                result?;
            },
            ToolExecutionPoll::Ready => {
                let Some(result) = active.execution.take_result() else {
                    state.active_tool = Some(active);
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "ready tool execution has no result",
                    ));
                };
                if let Err(error) = active.execution.shutdown() {
                    state.active_tool = Some(active);
                    return Err(map_tool_cleanup(error));
                }
                self.finish_tool(state, active.call, active.activity, result)?;
            },
        }
        Ok(())
    }

    fn publish_tool_progress(
        &mut self,
        state: &TurnState,
        active: &mut ActiveTool,
    ) -> Result<(), BackendFailure> {
        let Some(progress) = active.execution.take_progress() else {
            return Ok(());
        };
        // Cropped snapshots can split semantic material at the retained head/tail boundaries.
        if progress.truncated || progress.output.len() > self.config.maximum_tool_output_bytes {
            return Ok(());
        }
        let admitted = self
            .semantic_admission
            .as_ref()
            .expect("an executed local tool requires semantic admission")
            .admit_progress(active.call.definition(), &progress.output)
            .map_err(|_| {
                failure(
                    BackendFailureKind::Protocol,
                    "tool progress semantic admission was rejected",
                )
            })?;
        let Some(output) = admitted else {
            return Ok(());
        };
        if output.len() > self.config.maximum_tool_output_bytes {
            return Err(failure(
                BackendFailureKind::Protocol,
                "semantic admission returned oversized tool progress",
            ));
        }
        let arguments = state.delta.iter().rev().find_map(|item| match item {
            ModelReplayItem::FunctionCall {
                call_id, arguments, ..
            } if call_id == active.call.call_id() => from_str(arguments).ok(),
            _ => None,
        });
        let profile = ToolOutput {
            tool: active.call.definition().wire_name().to_owned(),
            server: None,
            arguments,
            result: Some(json!({"content":[{"type":"text","text":output}],"progress":true})),
            content_items: None,
            error: None,
            plain_text: output,
        };
        if let Some(snapshot) = profile.to_snapshot() {
            self.events.push_back(BackendEvent::ActivityUpdated {
                activity: active.activity,
                update: ActivityUpdate::TextSnapshot(snapshot),
            });
        }
        Ok(())
    }

    fn finish_tool_without_execution(
        &mut self,
        state: &mut TurnState,
        call: ValidatedToolCall,
        outcome: ToolExecutionOutcome,
        output: String,
    ) -> Result<(), BackendFailure> {
        let activity = self.next_activity(state.turn)?;
        self.events.push_back(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::ToolResult,
        });
        self.finish_tool(
            state,
            call,
            activity,
            ToolExecutionResult::new(outcome, output, false),
        )
    }

    fn finish_tool(
        &mut self,
        state: &mut TurnState,
        call: ValidatedToolCall,
        activity: ActivityRef,
        result: ToolExecutionResult,
    ) -> Result<(), BackendFailure> {
        let outcome = result.outcome();
        let truncated =
            result.truncated() || result.output().len() > self.config.maximum_tool_output_bytes;
        let output = bounded_output(
            result.output(),
            self.config.maximum_tool_output_bytes,
            result.truncated(),
        );
        let admitted_arguments = state.delta.iter().rev().find_map(|item| match item {
            ModelReplayItem::FunctionCall {
                call_id, arguments, ..
            } if call_id == call.call_id() => from_str(arguments).ok(),
            _ => None,
        });
        let native_edit_completed =
            is_native_edit(&call) && outcome == ToolExecutionOutcome::Completed;
        let publication_candidate = native_edit_completed
            && admitted_edit_spec(admitted_arguments.as_ref()).is_some_and(
                |(path, replacements)| {
                    native_edit_success_receipt_matches(
                        result.output(),
                        path,
                        replacements,
                        truncated,
                    )
                },
            );
        let retained = if native_edit_completed {
            None
        } else if let Some((text, truncated)) = result.retained_output() {
            let limit = self.config.maximum_retained_tool_output_bytes.unwrap_or(0);
            if text.len() > limit || self.config.maximum_retained_tool_output_bytes.is_none() {
                self.fail_tool_admission(
                    activity,
                    call.call_id().to_owned(),
                    call.definition().wire_name().to_owned(),
                    "tool retained output exceeds its configured bound",
                );
                return Err(failure(
                    BackendFailureKind::Protocol,
                    "tool retained output exceeds its configured bound",
                ));
            }
            let admitted = self
                .semantic_admission
                .as_ref()
                .expect("an executed local tool requires semantic admission")
                .admit_output(call.definition(), text);
            match admitted {
                Ok(text) if text.len() <= limit => Some((text, truncated)),
                _ => {
                    self.fail_tool_admission(
                        activity,
                        call.call_id().to_owned(),
                        call.definition().wire_name().to_owned(),
                        "tool retained output semantic admission was rejected",
                    );
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "tool retained output semantic admission was rejected",
                    ));
                },
            }
        } else {
            None
        };
        let output = match self
            .semantic_admission
            .as_ref()
            .expect("an executed local tool requires semantic admission")
            .admit_output(call.definition(), &output)
        {
            Ok(admitted) if admitted.len() <= self.config.maximum_tool_output_bytes => admitted,
            Ok(_) => {
                let message = "semantic admission returned oversized tool output";
                self.fail_tool_admission(
                    activity,
                    call.call_id().to_owned(),
                    call.definition().wire_name().to_owned(),
                    message,
                );
                return Err(failure(BackendFailureKind::Protocol, message));
            },
            Err(_) => {
                let message = "tool output semantic admission was rejected";
                self.fail_tool_admission(
                    activity,
                    call.call_id().to_owned(),
                    call.definition().wire_name().to_owned(),
                    message,
                );
                return Err(failure(BackendFailureKind::Protocol, message));
            },
        };
        let activity_outcome = match outcome {
            ToolExecutionOutcome::Completed => ActivityOutcome::Completed,
            ToolExecutionOutcome::Failed => {
                ActivityOutcome::Failed(Failure::new("tool execution failed"))
            },
            ToolExecutionOutcome::Interrupted => ActivityOutcome::Interrupted,
        };
        let replay_output = ModelReplayItem::FunctionCallOutput {
            call_id: call.call_id().to_owned(),
            output,
        };
        self.ensure_accumulated_replay_capacity(state, Some(&replay_output))?;
        let ModelReplayItem::FunctionCallOutput { output, .. } = &replay_output else {
            unreachable!("the replay output was constructed as a function result")
        };
        // Reuse admitted replay arguments, never the execution call's raw arguments.
        let arguments = admitted_arguments;
        let publication_path = if publication_candidate {
            admitted_edit_spec(arguments.as_ref()).map(|(path, _)| path.to_owned())
        } else {
            None
        };
        let publication_evidence = if publication_candidate {
            catch_unwind(AssertUnwindSafe(|| {
                self.semantic_admission.as_ref().and_then(|admission| {
                    edit_publication_evidence_snapshot(EditPublicationSnapshotContext {
                        call: &call,
                        arguments: arguments.as_ref(),
                        native_output: result.output(),
                        admitted_output: output,
                        truncated,
                        retained: result.retained_output(),
                        retention_limit: self.config.maximum_retained_tool_output_bytes,
                        admission: admission.as_ref(),
                    })
                })
            }))
            .ok()
            .flatten()
            .or_else(|| {
                publication_path.as_deref().and_then(|path| {
                    unavailable_publication_evidence(
                        path,
                        FilePublicationEvidenceUnavailableReason::GenerationFailed,
                    )
                })
            })
        } else {
            None
        };
        let status = match outcome {
            ToolExecutionOutcome::Completed => "completed",
            ToolExecutionOutcome::Failed => "failed",
            ToolExecutionOutcome::Interrupted => "interrupted",
        };
        let display_output = retained
            .as_ref()
            .map_or(output.as_str(), |(text, _)| text.as_str());
        let plain_text = if let (Some(path), Some(_)) =
            (publication_path.as_deref(), publication_evidence.as_ref())
        {
            format!(
                "{} · {}\n{status}\nPath: {path}\nResult:\n{display_output}",
                call.definition().wire_name(),
                call.call_id(),
            )
        } else {
            format!(
                "{} · {}\n{status}\nArguments:\n{}\nResult:\n{display_output}",
                call.definition().wire_name(),
                call.call_id(),
                arguments.as_ref().map_or_else(
                    || "(not available)".to_owned(),
                    |value| format!("{value:#}")
                )
            )
        };
        let mut profile = ToolOutput {
            tool: call.definition().wire_name().to_owned(),
            server: None,
            arguments,
            result: Some(json!({
                "content": [{"type": "text", "text": output}],
                "call_id": call.call_id(),
                "tool_id": call.definition().id().as_str(),
                "execution_host": self.tool_host.identity(),
                "outcome": status,
                "truncated": truncated,
                "isError": outcome != ToolExecutionOutcome::Completed,
            })),
            content_items: None,
            error: None,
            plain_text,
        };
        if let Some((_, truncated)) = &retained {
            profile.result.as_mut().expect("native result exists")["retainedOutput"] =
                json!({"truncated": truncated});
        }
        if let Some(evidence) = &publication_evidence {
            profile.result.as_mut().expect("native result exists")["publicationEvidence"] =
                json!(evidence);
        }
        let snapshot = if let Some(snapshot) = profile.to_snapshot() {
            snapshot
        } else if retained.is_some() {
            // The retained representation must never disappear into the small replay fallback.
            self.fail_tool_admission(
                activity,
                call.call_id().to_owned(),
                call.definition().wire_name().to_owned(),
                "tool retained output exceeds presentation capacity",
            );
            return Err(failure(
                BackendFailureKind::Protocol,
                "tool retained output exceeds presentation capacity",
            ));
        } else if let (Some(path), Some(_)) =
            (publication_path.as_deref(), publication_evidence.as_ref())
        {
            let unavailable = FilePublicationEvidence::unavailable(
                path,
                FilePublicationEvidenceUnavailableReason::SnapshotCapacity,
            )
            .and_then(|evidence| evidence.to_snapshot());
            if let Some(unavailable) = unavailable {
                profile.arguments = Some(json!({"path": path}));
                profile.plain_text = format!(
                    "{} · {}\n{status}\nPath: {path}\nPublication evidence unavailable: snapshot capacity.",
                    call.definition().wire_name(),
                    call.call_id(),
                );
                profile.result.as_mut().expect("native result exists")["publicationEvidence"] =
                    json!(unavailable);
            }
            profile.to_snapshot().unwrap_or_else(|| {
                json!({ "call_id": call.call_id(), "output": output }).to_string()
            })
        } else {
            json!({ "call_id": call.call_id(), "output": output }).to_string()
        };
        self.events.push_back(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(snapshot),
        });
        self.events.push_back(BackendEvent::ActivityFinished {
            activity,
            outcome: activity_outcome,
        });
        state.delta.push(replay_output);
        if state.pending_calls.is_empty() {
            self.events
                .push_back(BackendEvent::ContextActiveSuffixCompleted {
                    turn: state.turn,
                    items: state.delta.clone(),
                });
            state.start_next_round = true;
        } else {
            self.advance_tool_queue(state)?;
        }
        Ok(())
    }

    pub(super) fn respond_to_approval(
        &mut self,
        request: ActivityRequestRef,
        decision: ApprovalDecision,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if matches!(decision, ApprovalDecision::Offered(_)) {
            return Err(failure(
                BackendFailureKind::Unsupported,
                "offered approval choices are unsupported by the managed backend",
            ));
        }
        let mut state = self.turn.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Turn,
                "approval response has no active Turn",
            )
        })?;
        let Some((expected, pending)) = state.awaiting_approval.take() else {
            self.turn = Some(state);
            return Err(failure(
                BackendFailureKind::Turn,
                "no approval is awaiting a response",
            ));
        };
        if expected != request
            || !pending.approval.as_ref().is_some_and(|binding| {
                pending.plan.as_ref().is_some_and(|plan| {
                    binding.matches_plan(
                        state.turn,
                        &pending.call,
                        self.tool_host.identity(),
                        plan.identity(),
                    )
                })
            })
        {
            state.awaiting_approval = Some((expected, pending));
            self.turn = Some(state);
            return Err(failure(
                BackendFailureKind::Turn,
                "approval response does not match the exact tool execution binding",
            ));
        }
        self.events.push_back(BackendEvent::ActivityFinished {
            activity: request.activity(),
            outcome: ActivityOutcome::Completed,
        });
        let response_activity = self.next_activity(state.turn)?;
        self.queue_activity_text(
            response_activity,
            ActivityKind::ApprovalResponse {
                request_id: request.request_id(),
            },
            format!("{decision:?}"),
            Some(ActivityOutcome::Completed),
        );
        match decision {
            ApprovalDecision::Offered(_) => {
                unreachable!("offered choices rejected before state mutation")
            },
            ApprovalDecision::Approved => state.ready_tool = Some(pending),
            ApprovalDecision::Declined => {
                if let Err(error) = self.finish_tool_without_execution(
                    &mut state,
                    pending.call,
                    ToolExecutionOutcome::Failed,
                    r#"{"error":"tool approval declined"}"#.to_owned(),
                ) {
                    if error.kind() == BackendFailureKind::ContextExhausted {
                        self.fail_or_exhaust_turn(&mut state, error);
                        return Ok(BackendCommandEvidence::None);
                    }
                    self.turn = Some(state);
                    return Err(error);
                }
            },
        }
        self.turn = Some(state);
        Ok(BackendCommandEvidence::None)
    }
}

fn running_tool_output_snapshot(state: &TurnState, active: &ActiveTool) -> Option<String> {
    let (arguments, admitted_text) = state.delta.iter().rev().find_map(|item| match item {
        ModelReplayItem::FunctionCall {
            call_id, arguments, ..
        } if call_id == active.call.call_id() => {
            Some((from_str::<Value>(arguments).ok()?, arguments))
        },
        _ => None,
    })?;
    ToolOutput {
        tool: active.call.definition().wire_name().to_owned(),
        server: None,
        arguments: Some(arguments),
        result: None,
        content_items: None,
        error: None,
        plain_text: format!(
            "{} · {}\nArguments:\n{admitted_text}",
            active.call.definition().wire_name(),
            active.call.call_id(),
        ),
    }
    .to_snapshot()
}

fn is_native_edit(call: &ValidatedToolCall) -> bool {
    call.definition().id().as_str() == "edit-file" && call.definition().wire_name() == "edit_file"
}

fn admitted_edit_spec(arguments: Option<&Value>) -> Option<(&str, usize)> {
    let arguments = arguments?;
    let path = arguments.get("path")?.as_str()?;
    let edits = arguments.get("edits")?.as_array()?;
    (!path.is_empty() && (1..=256).contains(&edits.len())).then_some((path, edits.len()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeEditSuccessReceipt {
    path: String,
    status: String,
    replacements: usize,
}

fn native_edit_success_receipt_matches(
    output: &str,
    path: &str,
    replacements: usize,
    truncated: bool,
) -> bool {
    if truncated {
        return false;
    }
    from_str::<NativeEditSuccessReceipt>(output).is_ok_and(|receipt| {
        receipt.path == path && receipt.status == "ok" && receipt.replacements == replacements
    })
}

fn unavailable_publication_evidence(
    path: &str,
    reason: FilePublicationEvidenceUnavailableReason,
) -> Option<String> {
    FilePublicationEvidence::unavailable(path, reason)?.to_snapshot()
}

struct EditPublicationSnapshotContext<'a> {
    call: &'a ValidatedToolCall,
    arguments: Option<&'a Value>,
    native_output: &'a str,
    admitted_output: &'a str,
    truncated: bool,
    retained: Option<(&'a str, bool)>,
    retention_limit: Option<usize>,
    admission: &'a dyn ToolSemanticAdmission,
}

fn edit_publication_evidence_snapshot(
    context: EditPublicationSnapshotContext<'_>,
) -> Option<String> {
    let EditPublicationSnapshotContext {
        call,
        arguments,
        native_output,
        admitted_output,
        truncated,
        retained,
        retention_limit,
        admission,
    } = context;
    if !is_native_edit(call) {
        return None;
    }
    let (path, replacements) = admitted_edit_spec(arguments)?;
    if !native_edit_success_receipt_matches(native_output, path, replacements, truncated) {
        return None;
    }
    let Some(retention_limit) = retention_limit else {
        return unavailable_publication_evidence(
            path,
            FilePublicationEvidenceUnavailableReason::Disabled,
        );
    };
    if admitted_output != native_output {
        return unavailable_publication_evidence(
            path,
            FilePublicationEvidenceUnavailableReason::SemanticAdmission,
        );
    }
    let Some((snapshot, retained_truncated)) = retained else {
        return unavailable_publication_evidence(
            path,
            FilePublicationEvidenceUnavailableReason::InvalidEvidence,
        );
    };
    if retained_truncated {
        return unavailable_publication_evidence(
            path,
            FilePublicationEvidenceUnavailableReason::InvalidEvidence,
        );
    }
    if snapshot.len() > retention_limit
        || snapshot.len() > FilePublicationEvidence::MAX_SNAPSHOT_BYTES
    {
        return unavailable_publication_evidence(
            path,
            FilePublicationEvidenceUnavailableReason::OverBound,
        );
    }
    let Some(evidence) = FilePublicationEvidence::from_snapshot(snapshot) else {
        return unavailable_publication_evidence(
            path,
            FilePublicationEvidenceUnavailableReason::InvalidEvidence,
        );
    };
    if evidence.path() != path {
        return unavailable_publication_evidence(
            path,
            FilePublicationEvidenceUnavailableReason::InvalidEvidence,
        );
    }
    match evidence.state() {
        FilePublicationEvidenceState::Unavailable { .. } => Some(snapshot.to_owned()),
        FilePublicationEvidenceState::Complete { before, after } => {
            let before_admitted = admission.admit_output(call.definition(), before);
            let after_admitted = admission.admit_output(call.definition(), after);
            if !before_admitted
                .as_ref()
                .is_ok_and(|admitted| admitted == before)
                || !after_admitted
                    .as_ref()
                    .is_ok_and(|admitted| admitted == after)
            {
                return unavailable_publication_evidence(
                    path,
                    FilePublicationEvidenceUnavailableReason::SemanticAdmission,
                );
            }
            Some(snapshot.to_owned())
        },
    }
}

fn bounded_output(output: &str, limit: usize, already_truncated: bool) -> String {
    if output.len() <= limit && !already_truncated {
        return output.to_owned();
    }
    if limit <= TOOL_TRUNCATION_MARKER.len() {
        let mut end = limit;
        while end > 0 && !TOOL_TRUNCATION_MARKER.is_char_boundary(end) {
            end -= 1;
        }
        return TOOL_TRUNCATION_MARKER[..end].to_owned();
    }
    let mut end = output.len().min(limit - TOOL_TRUNCATION_MARKER.len());
    while end > 0 && !output.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{TOOL_TRUNCATION_MARKER}", &output[..end])
}

fn map_tool_turn(_error: yo_core::ToolExecutionError) -> BackendFailure {
    failure(BackendFailureKind::Turn, "tool execution failed")
}

pub(super) fn tool_validation_failure(kind: ToolValidationFailure, message: &str) -> Failure {
    Failure::new(message)
        .with_code(kind.code())
        .expect("tool validation codes are stable ASCII identifiers")
}

pub(super) fn durable_tool_validation_message(kind: ToolValidationFailure) -> &'static str {
    match kind {
        ToolValidationFailure::InvalidIdentity => "tool identity is invalid",
        ToolValidationFailure::ArgumentLimit => "tool arguments exceed the admitted limit",
        ToolValidationFailure::InvalidJson => "tool arguments are not valid JSON",
        ToolValidationFailure::SchemaMismatch => "tool arguments do not match the admitted schema",
        ToolValidationFailure::UnknownTool => "the requested tool is not admitted",
        ToolValidationFailure::DuplicateIdentity => "tool call identity was already used",
        ToolValidationFailure::Unavailable => "the requested tool is unavailable",
        ToolValidationFailure::ApprovalMismatch => "tool approval does not match the request",
        ToolValidationFailure::SemanticAdmission => "tool semantic admission was rejected",
    }
}

#[cfg(test)]
mod tests;
