use serde_json::Value;
use yo_core::{
    ActivityOutcome, BackendEvent, BackendFailure, BackendOutcomeEvidence, Failure, TurnOutcome,
};

use super::super::state::Backend;
use crate::{protocol, transport::JsonPeer};

pub(super) fn prompt_completed<P: JsonPeer>(
    backend: &mut Backend<P>,
    response_id: u64,
    result: &Value,
    usage_receipt: impl FnOnce(&Value, u64) -> Result<Option<Value>, BackendFailure>,
) -> Result<Option<BackendEvent>, BackendFailure> {
    backend.prompt_completed(response_id, result, usage_receipt)
}

impl<P: JsonPeer> Backend<P> {
    fn prompt_completed<F: FnOnce(&Value, u64) -> Result<Option<Value>, BackendFailure>>(
        &mut self,
        response_id: u64,
        result: &Value,
        usage_receipt: F,
    ) -> Result<Option<BackendEvent>, BackendFailure> {
        let prompt = self.prompt.take().ok_or_else(|| {
            protocol::protocol_failure("Grok ACP prompt response has no active Turn")
        })?;
        if prompt.request_id != response_id {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP completed prompt {response_id} instead of {}",
                prompt.request_id
            )));
        }
        if !self.approvals.is_empty() {
            return Err(protocol::protocol_failure(
                "Grok ACP completed a prompt with an unresolved permission request",
            ));
        }
        if !self.inputs.is_empty() {
            return Err(protocol::protocol_failure(
                "Grok ACP completed a prompt with an unresolved user question request",
            ));
        }
        if self.pending_probe.is_some() {
            self.cancel_secret_probe();
            return Err(protocol::protocol_failure(
                "Grok ACP completed a prompt with an unresolved secret-entry probe",
            ));
        }
        self.cancel_secret_probe();
        let stop_reason = protocol::string_at(result, &["stopReason"])?;
        if prompt.interrupt_requested && stop_reason != "cancelled" {
            return Err(protocol::protocol_failure(format!(
                "Grok ACP returned `{stop_reason}` after session/cancel"
            )));
        }
        let outcome = match stop_reason {
            "end_turn" => TurnOutcome::Completed,
            "cancelled" => TurnOutcome::Interrupted,
            "max_tokens" => TurnOutcome::Failed(Failure::new("Grok reached its token limit")),
            "max_turn_requests" => {
                TurnOutcome::Failed(Failure::new("Grok reached its agent request limit"))
            },
            "refusal" => TurnOutcome::Failed(Failure::new("Grok refused the request")),
            other => {
                return Err(protocol::protocol_failure(format!(
                    "Grok ACP returned unsupported stop reason `{other}`"
                )));
            },
        };
        let usage_receipt = usage_receipt(result, response_id)?;
        let activity_outcome = match &outcome {
            TurnOutcome::Completed => ActivityOutcome::Completed,
            TurnOutcome::Interrupted => ActivityOutcome::Interrupted,
            TurnOutcome::Failed(failure) => ActivityOutcome::Failed(failure.clone()),
        };
        let mut activities = self
            .messages
            .drain()
            .map(|(_, binding)| binding.activity)
            .chain(self.tools.drain().flat_map(|(_, binding)| {
                (!binding.finished)
                    .then_some(binding.activity)
                    .into_iter()
                    .chain(
                        (!binding.finished)
                            .then_some(binding.result_activity)
                            .flatten(),
                    )
            }))
            .chain(self.approvals.drain().map(|(_, binding)| binding.activity))
            .chain(self.inputs.drain().map(|(_, binding)| binding.activity))
            .collect::<Vec<_>>();
        self.wire_approvals.clear();
        self.wire_inputs.clear();
        activities.sort_unstable();
        activities.dedup();
        self.pending_events
            .extend(
                activities
                    .into_iter()
                    .map(|activity| BackendEvent::ActivityFinished {
                        activity,
                        outcome: activity_outcome.clone(),
                    }),
            );
        if let Some(receipt) = usage_receipt {
            self.queue_usage_activity(prompt.turn, receipt)?;
        }
        let turn_finished = if outcome == TurnOutcome::Completed && self.load_session {
            BackendEvent::ResumableTurnFinished {
                turn: prompt.turn,
                evidence: BackendOutcomeEvidence::without_identity(),
            }
        } else {
            BackendEvent::TurnFinished {
                turn: prompt.turn,
                outcome,
            }
        };
        self.pending_events.push_back(turn_finished);
        Ok(self.pending_events.pop_front())
    }
}
