//! Turn admission, completion, interruption, and resource cleanup.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::json;
use yo_core::{
    ActivityOutcome, ApiDialect, BackendCommandEvidence, BackendEvent, BackendFailure,
    BackendFailureKind, BackendIdentity, BackendOutcomeEvidence, Failure, ModelReplayDelta,
    ModelReplayItem, ModelReplayRole, TurnOutcome, TurnRef,
};

use super::{
    CONTEXT_EXHAUSTED_CODE, NativeModelBackend, TurnState, failure,
    replay::is_replay_capacity_error,
};

impl NativeModelBackend {
    pub(super) fn start_turn(
        &mut self,
        turn: TurnRef,
        input: String,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        if self.context_exhausted {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: this binding cannot admit another model request",
            ));
        }
        if self.session != Some(turn.session_id())
            || self.turn.is_some()
            || self.idle_compaction.is_some()
        {
            return Err(failure(
                BackendFailureKind::Session,
                "native backend requires its bound idle Session before starting a Turn",
            ));
        }
        let delta = vec![ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: input,
            refusal: None,
        }];
        let mut state = TurnState {
            turn,
            round: 0,
            delta,
            stream: None,
            response_id: None,
            assistant_activities: BTreeMap::new(),
            reasoning_activities: HashMap::new(),
            call_activities: HashMap::new(),
            seen_call_ids: self
                .replay
                .items()
                .iter()
                .filter_map(|item| match item {
                    ModelReplayItem::FunctionCall { call_id, .. } => Some(call_id.clone()),
                    _ => None,
                })
                .collect(),
            round_message_items: BTreeSet::new(),
            round_messages: BTreeMap::new(),
            round_refusals: BTreeMap::new(),
            round_replay: BTreeMap::new(),
            pending_calls: BTreeMap::new(),
            active_tool: None,
            ready_tool: None,
            dispatch_tool: None,
            awaiting_approval: None,
            start_next_round: false,
            compaction: None,
            compaction_attempted: false,
        };
        let request_started = match self.start_model_round(&mut state) {
            Ok(()) => {
                let request_started = state.compaction.is_none();
                self.turn = Some(state);
                request_started
            },
            Err(error) if error.kind() == BackendFailureKind::ContextExhausted => {
                self.context_exhausted = true;
                self.exhaust_turn(&mut state, error.to_string());
                false
            },
            Err(error) => return Err(error),
        };
        if !request_started {
            return Ok(BackendCommandEvidence::None);
        }
        Ok(BackendCommandEvidence::RequestAccepted(
            self.request_evidence(turn),
        ))
    }

    pub(super) fn complete_turn(&mut self, state: &mut TurnState) -> Result<(), BackendFailure> {
        let delta = ModelReplayDelta::new(
            self.replay
                .contract()
                .is_none()
                .then(|| self.contract.clone()),
            std::mem::take(&mut state.delta),
        );
        let completed_group = delta.items().to_vec();
        if let Err(message) = self.replay.apply(&delta) {
            if is_replay_capacity_error(message) {
                self.context_exhausted = true;
                self.events.push_back(BackendEvent::TurnFinished {
                    turn: state.turn,
                    outcome: TurnOutcome::Completed,
                });
                self.turn = None;
                return Ok(());
            }
            return Err(failure(BackendFailureKind::Turn, message));
        }
        self.replay_groups.push(completed_group);
        self.events.push_back(BackendEvent::ResumableTurnFinished {
            turn: state.turn,
            evidence: BackendOutcomeEvidence::with_identity(BackendIdentity::new(
                match self.binding.api_dialect() {
                    ApiDialect::OpenAiResponses => "responses.response-id/v1",
                    ApiDialect::OpenAiChatCompletions => "chat-completions.response-id/v1",
                    ApiDialect::KimiChatCompletions => "kimi-chat-completions.response-id/v1",
                },
                state
                    .response_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
            ))
            .with_replay(delta),
        });
        self.turn = None;
        Ok(())
    }

    fn cleanup_turn_resources(&mut self, state: &mut TurnState) -> Vec<String> {
        let mut diagnostics = Vec::new();
        if let Some(mut stream) = state.stream.take() {
            stream.cancel();
            if stream.shutdown().is_err() {
                diagnostics.push("response cleanup failed".to_owned());
            }
        }
        match self.shared_stop.response.lock() {
            Ok(mut response) => *response = None,
            Err(_) => diagnostics.push("native stop state is poisoned".to_owned()),
        }
        if let Some(mut active) = state.active_tool.take() {
            active.execution.cancel();
            if active.execution.shutdown().is_err() {
                diagnostics.push("tool execution cleanup failed".to_owned());
            }
        }
        diagnostics
    }

    pub(super) fn fail_turn(&mut self, state: &mut TurnState, mut message: String) {
        let active_tool = state
            .active_tool
            .as_ref()
            .map(|active| (active.activity, active.call.call_id().to_owned()));
        let diagnostics = self.cleanup_turn_resources(state);
        if !diagnostics.is_empty() {
            message.push_str("; ");
            message.push_str(&diagnostics.join("; "));
        }
        for activity in self.projected_open_activities() {
            if let Some((active_activity, call_id)) = active_tool.as_ref()
                && *active_activity == activity
            {
                self.events.push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: yo_core::ActivityUpdate::TextSnapshot(
                        json!({
                            "call_id": call_id,
                            "error": "execution failed or was cancelled; effect may be uncertain",
                        })
                        .to_string(),
                    ),
                });
            }
            self.events.push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Failed(Failure::new(message.clone())),
            });
        }
        self.events.push_back(BackendEvent::TurnFinished {
            turn: state.turn,
            outcome: TurnOutcome::Failed(Failure::new(message)),
        });
        self.turn = None;
    }

    pub(super) fn fail_or_exhaust_turn(&mut self, state: &mut TurnState, error: BackendFailure) {
        if error.kind() == BackendFailureKind::ContextExhausted {
            self.context_exhausted = true;
            self.exhaust_turn(state, error.to_string());
        } else {
            self.fail_turn(state, error.to_string());
        }
    }

    pub(super) fn exhaust_turn(&mut self, state: &mut TurnState, mut message: String) {
        let diagnostics = self.cleanup_turn_resources(state);
        if !diagnostics.is_empty() {
            message.push_str("; ");
            message.push_str(&diagnostics.join("; "));
        }
        let failure = context_exhausted_failure(message);
        for activity in self.projected_open_activities() {
            self.events.push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Failed(failure.clone()),
            });
        }
        self.events.push_back(BackendEvent::TurnFinished {
            turn: state.turn,
            outcome: TurnOutcome::Failed(failure),
        });
        self.turn = None;
    }

    pub(super) fn interrupt(
        &mut self,
        turn: TurnRef,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let Some(mut state) = self.turn.take() else {
            return Err(failure(
                BackendFailureKind::Turn,
                "no active Turn to interrupt",
            ));
        };
        if state.turn != turn {
            self.turn = Some(state);
            return Err(failure(
                BackendFailureKind::Turn,
                "interrupt names a different Turn",
            ));
        }
        let interrupted_call = state
            .active_tool
            .as_ref()
            .map(|active| (active.activity, active.call.call_id().to_owned()));
        let cleanup_diagnostics = self.cleanup_turn_resources(&mut state);
        self.events.clear();
        let open_activities = std::mem::take(&mut self.open_activities)
            .into_iter()
            .collect::<BTreeSet<_>>();
        for activity in open_activities {
            if let Some((interrupted_activity, call_id)) = interrupted_call.as_ref()
                && *interrupted_activity == activity
            {
                self.events.push_back(BackendEvent::ActivityUpdated {
                    activity,
                    update: yo_core::ActivityUpdate::TextSnapshot(
                        json!({
                            "call_id": call_id,
                            "error": "execution interrupted; effect may be uncertain",
                        })
                        .to_string(),
                    ),
                });
            }
            self.events.push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: if cleanup_diagnostics.is_empty() {
                    ActivityOutcome::Interrupted
                } else {
                    ActivityOutcome::Failed(Failure::new(cleanup_diagnostics.join("; ")))
                },
            });
        }
        self.events.push_back(BackendEvent::TurnFinished {
            turn,
            outcome: if cleanup_diagnostics.is_empty() {
                TurnOutcome::Interrupted
            } else {
                TurnOutcome::Failed(Failure::new(cleanup_diagnostics.join("; ")))
            },
        });
        Ok(BackendCommandEvidence::None)
    }
}

fn context_exhausted_failure(message: impl Into<String>) -> Failure {
    Failure::new(message)
        .with_code(CONTEXT_EXHAUSTED_CODE)
        .expect("the context exhaustion code is stable ASCII")
}
