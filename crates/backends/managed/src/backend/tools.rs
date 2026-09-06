//! Serial tool approval, execution, admitted output, and replay handoff.

use serde_json::json;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ApprovalDecision,
    BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind, Failure,
    ModelReplayItem, ToolApprovalBinding, ToolApprovalRequirement, ToolExecutionOutcome,
    ToolExecutionPoll, ToolExecutionRequest, ToolValidationFailure, ValidatedToolCall,
};

use super::{
    ActiveTool, NativeModelBackend, TOOL_TRUNCATION_MARKER, TurnState, failure, map_tool_cleanup,
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
        if pending.call.definition().approval() == ToolApprovalRequirement::Required {
            let activity = self.next_activity(state.turn)?;
            let request = ActivityRequestRef::new(activity, self.next_request()?);
            let binding =
                ToolApprovalBinding::new(state.turn, &pending.call, self.tool_host.identity());
            let approval_text = json!({
                "call_id": pending.call.call_id(),
                "tool_id": pending.call.definition().id().as_str(),
                "argument_digest": binding.argument_digest_hex(),
                "effect": format!("{:?}", binding.effect()),
                "execution_host": binding.execution_host(),
            })
            .to_string();
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
        state.ready_tool = Some(pending.call);
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
            update: yo_core::ActivityUpdate::TextSnapshot(
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
        call: ValidatedToolCall,
        activity: ActivityRef,
    ) -> Result<(), BackendFailure> {
        let request = ToolExecutionRequest {
            turn: state.turn,
            call: call.clone(),
            maximum_output_bytes: self.config.maximum_tool_output_bytes,
            absolute_execution_timeout: self.config.absolute_tool_execution_timeout,
        };
        match self.tool_host.start(request) {
            Ok(execution) => {
                state.active_tool = Some(ActiveTool {
                    call,
                    activity,
                    execution,
                });
            },
            Err(_) => self.finish_tool(
                state,
                call,
                activity,
                ToolExecutionOutcome::Failed,
                "tool execution failed".to_owned(),
            )?,
        }
        Ok(())
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
            ToolExecutionPoll::Pending => state.active_tool = Some(active),
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
                self.finish_tool(
                    state,
                    active.call,
                    active.activity,
                    result.outcome(),
                    bounded_output(
                        result.output(),
                        self.config.maximum_tool_output_bytes,
                        result.truncated(),
                    ),
                )?;
            },
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
        self.finish_tool(state, call, activity, outcome, output)
    }

    fn finish_tool(
        &mut self,
        state: &mut TurnState,
        call: ValidatedToolCall,
        activity: ActivityRef,
        outcome: ToolExecutionOutcome,
        output: String,
    ) -> Result<(), BackendFailure> {
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
        self.events.push_back(BackendEvent::ActivityUpdated {
            activity,
            update: yo_core::ActivityUpdate::TextSnapshot(
                json!({ "call_id": call.call_id(), "output": output }).to_string(),
            ),
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
                binding.matches(state.turn, &pending.call, self.tool_host.identity())
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
            ApprovalDecision::Approved => state.ready_tool = Some(pending.call),
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
