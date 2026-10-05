//! Turn admission, completion, interruption, and resource cleanup.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    mem,
};

use serde_json::json;
use yo_core::{
    ActivityOutcome, ApiDialect, BackendCommandEvidence, BackendEvent, BackendFailure,
    BackendFailureKind, BackendIdentity, BackendOutcomeEvidence, Failure, ModelReplayDelta,
    ModelReplayItem, TurnOutcome, TurnRef, UserInput,
};

use super::{
    CONTEXT_EXHAUSTED_CODE, NativeModelBackend, TurnState, failure,
    replay::is_replay_capacity_error,
};

impl NativeModelBackend {
    pub(super) fn start_turn(
        &mut self,
        turn: TurnRef,
        input: ModelReplayItem,
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
        let delta = vec![input];
        let mut state = TurnState {
            turn,
            round: 0,
            delta,
            closed_source: None,
            open_group_effect_attempted: false,
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
            question_call_start: None,
            pending_question: None,
            awaiting_question: None,
            prepared_question: None,
            secret_call_start: None,
            pending_secret_call: None,
            awaiting_secret_input: None,
            prepared_secret_request: None,
            prepared_steer: None,
            armed_steers: Default::default(),
            armed_steer_encoded_bytes: 0,
            terminal_secret_request: false,
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

    pub(super) fn prepare_steer(
        &mut self,
        turn: TurnRef,
        input: UserInput,
    ) -> Result<BackendCommandEvidence, BackendFailure> {
        let Some(state) = self.turn.as_ref() else {
            return Err(failure(
                BackendFailureKind::CommandRejected,
                "the exact Turn is no longer active in the native backend",
            ));
        };
        if state.turn != turn {
            return Err(failure(
                BackendFailureKind::CommandRejected,
                "the exact Turn is no longer active in the native backend",
            ));
        }
        if state.awaiting_secret_input.is_some()
            || state.prepared_secret_request.is_some()
            || state.terminal_secret_request
        {
            return Err(failure(
                BackendFailureKind::CommandRejected,
                "the native secret interaction does not admit steering",
            ));
        }
        if state.prepared_steer.is_some() || state.prepared_question.is_some() {
            return Err(failure(
                BackendFailureKind::CommandRejected,
                "a native Turn input is already awaiting durable commit",
            ));
        }

        let item = input.model_replay_item();
        if !self.steer_fits_pending_replay(state, &item)? {
            return Err(failure(
                BackendFailureKind::CommandRejected,
                "native Turn input exceeds the pending replay capacity",
            ));
        }
        self.turn
            .as_mut()
            .expect("the exact Turn remained active during admission")
            .prepared_steer = Some(item);
        Ok(BackendCommandEvidence::SubmissionPrepared)
    }

    pub(super) fn commit_prepared_steer(&mut self) -> Result<(), BackendFailure> {
        let state = self.turn.as_mut().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "prepared native Turn input lost its active Turn before commit",
            )
        })?;
        let item = state.prepared_steer.take().ok_or_else(|| {
            failure(
                BackendFailureKind::Protocol,
                "no native Turn input is prepared for durable commit",
            )
        })?;
        state.armed_steer_encoded_bytes = state
            .armed_steer_encoded_bytes
            .checked_add(item.encoded_len())
            .ok_or_else(|| {
                failure(
                    BackendFailureKind::ContextExhausted,
                    "native Turn input exceeds the pending replay capacity",
                )
            })?;
        if let Some(source) = state.closed_source.as_mut() {
            source.push(item.clone());
        }
        state.armed_steers.push_back(item);
        Ok(())
    }

    pub(super) fn abort_prepared_steer(&mut self) -> bool {
        let Some(state) = self.turn.as_mut() else {
            return false;
        };
        state.prepared_steer.take().is_some()
    }

    pub(super) fn complete_turn(&mut self, state: &mut TurnState) -> Result<(), BackendFailure> {
        let delta = ModelReplayDelta::new(
            self.replay
                .contract()
                .is_none()
                .then(|| self.contract.clone()),
            mem::take(&mut state.delta),
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
        state.prepared_secret_request = None;
        diagnostics
    }

    // 기존 checkpoint의 직렬 반환 경계로 Core 확정 후의 호출에서만 승격합니다.
    pub(super) fn promote_failure_context(&mut self) {
        if self.events.is_empty()
            && let Some((replay, group)) = self.pending_failure_context.take()
        {
            self.replay = replay;
            if let Some(group) = group {
                self.replay_groups.push(group);
            }
        }
    }

    pub(super) fn reject_tool_arguments(&mut self, state: &mut TurnState, message: String) {
        let candidate = (|| {
            if self.replay_profile != yo_core::ReplayProfile::SemanticOnly
                || state.open_group_effect_attempted
                || state.terminal_secret_request
                || !state.armed_steers.is_empty()
                || state.prepared_steer.is_some()
            {
                return None;
            }
            let items = state.closed_source.as_ref()?;
            let delta = (!items.is_empty()).then(|| {
                ModelReplayDelta::new(
                    self.replay
                        .contract()
                        .is_none()
                        .then(|| self.contract.clone()),
                    items.clone(),
                )
            });
            let mut replay = self.replay.clone();
            if let Some(delta) = &delta {
                replay.apply(delta).ok()?;
            }
            Some((replay, delta))
        })();
        let Some((replay, delta)) = candidate else {
            self.fail_turn(state, message);
            return;
        };
        let diagnostics = self.cleanup_turn_resources(state);
        if !diagnostics.is_empty() {
            self.fail_turn(state, format!("{message}; {}", diagnostics.join("; ")));
            return;
        }
        for activity in self.projected_open_activities() {
            self.events.push_back(BackendEvent::ActivityFinished {
                activity,
                outcome: ActivityOutcome::Failed(Failure::new(message.clone())),
            });
        }
        let group = delta.as_ref().map(|delta| delta.items().to_vec());
        self.pending_failure_context = Some((replay, group));
        self.events
            .push_back(BackendEvent::LocalArgumentRejectionPrepared {
                turn: state.turn,
                failure: Failure::new(message),
                replay: delta,
            });
        self.turn = None;
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
        let open_activities = mem::take(&mut self.open_activities)
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
