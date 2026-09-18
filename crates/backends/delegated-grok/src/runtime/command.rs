use serde_json::json;
use yo_core::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, ActivityUpdate,
    AgentCommand, ApprovalDecision, BackendCommandEvidence, BackendFailure, BackendFailureKind,
    BackendIdentity, BackendRequestEvidence, TurnRef,
};

use super::state::Backend;
use crate::{protocol, transport::JsonPeer};

impl<P: JsonPeer> Backend<P> {
    pub(super) fn execute_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if let AgentCommand::StartTurn { input, .. } | AgentCommand::SteerTurn { input, .. } =
            &command
            && !input.images().is_empty()
        {
            return Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Grok ACP image input requires negotiated protocol and selected-model capability",
            ));
        }
        match command {
            AgentCommand::CreateSession { session_id } => self.create_session(session_id),
            AgentCommand::StartTurn { turn, input } => {
                self.start_turn(turn, input.into_model_input())
            },
            AgentCommand::SteerTurn { .. } => Err(BackendFailure::new(
                BackendFailureKind::Unsupported,
                "Grok ACP v1 does not support steering an active Turn",
            )),
            AgentCommand::InterruptTurn { turn } => self.interrupt_turn(turn),
            AgentCommand::RespondToActivity { request, response } => {
                self.respond_to_activity(request, response)
            },
            AgentCommand::CompactContext { .. } => Err(BackendFailure::new(
                BackendFailureKind::CommandRejected,
                "Grok delegated Sessions do not use Yo-managed context compaction",
            )),
        }
    }

    fn start_turn(
        &mut self,
        turn: TurnRef,
        input: String,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if self.prompt.is_some() {
            return Err(protocol::protocol_failure(
                "Grok ACP already has an active prompt",
            ));
        }
        let session_id = self.session_id(turn.session_id())?.to_owned();
        let request_id = self.client.begin_prompt(
            json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": input }],
            }),
            &session_id,
        )?;
        self.prompt = Some(super::state::PromptBinding {
            request_id,
            turn,
            interrupt_requested: false,
        });
        Ok(BackendCommandEvidence::RequestAccepted(
            BackendRequestEvidence::new(
                "grok.acp/session-prompt/v1",
                BackendIdentity::new("grok.acp/json-rpc-request/v1", request_id.to_string()),
                BackendIdentity::new(
                    "grok.acp/accepted-prompt/v1",
                    json!({ "jsonRpcId": request_id, "sessionId": session_id }).to_string(),
                ),
            ),
        ))
    }

    fn interrupt_turn(&mut self, turn: TurnRef) -> Result<BackendCommandEvidence, BackendFailure> {
        let session_id = self.session_id(turn.session_id())?.to_owned();
        let prompt = self
            .prompt
            .as_mut()
            .filter(|prompt| prompt.turn == turn)
            .ok_or_else(|| protocol::protocol_failure("Grok active prompt was not found"))?;
        self.client
            .notify("session/cancel", json!({ "sessionId": session_id }))?;
        prompt.interrupt_requested = true;
        let approvals = self.approvals.drain().collect::<Vec<_>>();
        self.wire_approvals.clear();
        for (_, approval) in approvals {
            self.client.respond(
                approval.wire_id,
                json!({ "outcome": { "outcome": "cancelled" } }),
            )?;
            self.pending_events
                .push_back(yo_core::BackendEvent::ActivityFinished {
                    activity: approval.activity,
                    outcome: ActivityOutcome::Interrupted,
                });
        }
        let inputs = self.inputs.drain().collect::<Vec<_>>();
        self.wire_inputs.clear();
        for (_, input) in inputs {
            self.client
                .respond(input.wire_id, json!({ "outcome": "cancelled" }))?;
            self.pending_events
                .push_back(yo_core::BackendEvent::ActivityFinished {
                    activity: input.activity,
                    outcome: ActivityOutcome::Interrupted,
                });
        }
        Ok(BackendCommandEvidence::None)
    }

    fn respond_to_activity(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if self.inputs.contains_key(&request) {
            return self.respond_to_question(request, response);
        }
        let approval = self.approvals.get(&request).cloned().ok_or_else(|| {
            protocol::protocol_failure("activity response has no matching Grok request")
        })?;
        let option_id = match response {
            ActivityResponse::Approval(ApprovalDecision::Approved) => approval.allow_option,
            ActivityResponse::Approval(ApprovalDecision::Declined) => approval.reject_option,
            ActivityResponse::Approval(ApprovalDecision::Offered(ordinal)) => ordinal
                .checked_sub(1)
                .and_then(|index| approval.offered.get(index as usize))
                .and_then(Clone::clone)
                .ok_or_else(|| protocol::protocol_failure("Grok approval choice is unavailable"))?,
            ActivityResponse::UserInput(_)
            | ActivityResponse::QuestionAnswer { .. }
            | ActivityResponse::PreviousQuestion { .. }
            | ActivityResponse::SecretInput(_)
            | ActivityResponse::SecretInputSubmitted => {
                return Err(BackendFailure::new(
                    BackendFailureKind::Unsupported,
                    "Grok user-input responses are not enabled in the ACP adapter",
                ));
            },
        };
        let selected = approval
            .offered
            .iter()
            .position(|id| id.as_ref() == Some(&option_id))
            .and_then(|index| approval.profile.choices.get(index))
            .expect("validated approval choice remains bound");
        let receipt = format!(
            "Selected: {}\n{}\nResponse sent to agent.",
            selected.label, selected.description
        );
        self.client.respond(
            approval.wire_id,
            json!({
                "outcome": { "outcome": "selected", "optionId": option_id }
            }),
        )?;
        self.approvals.remove(&request);
        self.wire_approvals.retain(|_, bound| *bound != request);
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityFinished {
                activity: approval.activity,
                outcome: ActivityOutcome::Completed,
            });
        let response_activity = self.next_activity(request.activity().turn())?;
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityStarted {
                activity: response_activity,
                kind: ActivityKind::ApprovalResponse {
                    request_id: request.request_id(),
                },
            });
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityUpdated {
                activity: response_activity,
                update: ActivityUpdate::TextSnapshot(receipt),
            });
        self.pending_events
            .push_back(yo_core::BackendEvent::ActivityFinished {
                activity: response_activity,
                outcome: ActivityOutcome::Completed,
            });
        Ok(BackendCommandEvidence::None)
    }
}
