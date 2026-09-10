//! Exact model-request construction, token counting, and bounded output-cap admission.

use yo_core::{
    ApiDialect, BackendFailure, BackendFailureKind, BackendIdentity, BackendRequestEvidence,
    ModelCacheAffinityHint, ModelConnectorCancellation, ModelConnectorInputItem,
    ModelConnectorInputRole, ModelConnectorRequest, ModelReplayDelta, RequestToolExposure,
    SessionId, TurnRef,
};

use super::{
    InputCount, NativeModelBackend, TurnState, failure, map_connector_turn, replay::replay_input,
};

impl NativeModelBackend {
    pub(super) fn request_evidence(&self, turn: TurnRef) -> BackendRequestEvidence {
        let (request_schema, endpoint_schema) = match self.binding.api_dialect() {
            ApiDialect::OpenAiResponses => (
                "openai.responses/yo-managed-turn/v1",
                "responses.endpoint/v1",
            ),
            ApiDialect::OpenAiChatCompletions => (
                "openai.chat-completions/yo-managed-turn/v1",
                "chat-completions.endpoint/v1",
            ),
            ApiDialect::KimiChatCompletions => (
                "kimi.chat-completions/yo-managed-turn/v1",
                "kimi-chat-completions.endpoint/v1",
            ),
        };
        BackendRequestEvidence::new(
            request_schema,
            BackendIdentity::new(endpoint_schema, self.connector.request_url()),
            BackendIdentity::new(
                "yo.turn/v1",
                format!("{}:{}", turn.session_id(), turn.turn_id().get()),
            ),
        )
    }

    pub(super) fn start_model_round(
        &mut self,
        state: &mut TurnState,
    ) -> Result<(), BackendFailure> {
        if state.round >= self.config.maximum_model_rounds {
            return Err(failure(
                BackendFailureKind::Turn,
                "native model loop exceeded its model-round limit",
            ));
        }
        self.ensure_pending_replay_capacity(state)?;
        let mut items = Vec::new();
        items.push(ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::System,
            content: self.contract.system_prompt().to_owned(),
            refusal: None,
        });
        items.extend(self.replay.items().iter().map(replay_input));
        items.extend(state.delta.iter().map(replay_input));
        let tool_exposure =
            if self.tool_exposure_enabled {
                RequestToolExposure::enabled(self.registry.function_tools().map_err(|error| {
                    failure(BackendFailureKind::Initialization, error.to_string())
                })?)
            } else {
                RequestToolExposure::disabled()
            };
        let replay_budget = ModelReplayDelta::replay_budget(
            self.replay.contract().is_none().then_some(&self.contract),
            state.delta.iter(),
        )
        .ok_or_else(|| {
            failure(
                BackendFailureKind::ContextExhausted,
                "model replay delta capacity was exhausted before request dispatch",
            )
        })?;
        let input_limit = self.model_context.input_token_limit();
        let mut request_cap = self.model_context.max_output_tokens();
        let mut cap_adjustments = 0_u8;
        let request = loop {
            let request = match ModelConnectorRequest::new(
                items.clone(),
                tool_exposure.clone(),
                request_cap,
                self.config.reasoning_effort,
            ) {
                Ok(request) => request,
                Err(error) => {
                    self.observe_connector_failure(state.turn, &error);
                    return Err(map_connector_turn(error));
                },
            }
            .with_replay_budget(replay_budget)
            .with_cache_affinity_hint(ModelCacheAffinityHint::for_session(state.turn.session_id()));
            let input_count = self.count_request_input(&request)?;
            let input_tokens = input_count.planning_tokens();
            match request_cap {
                Some(cap)
                    if input_tokens
                        .checked_add(cap)
                        .is_some_and(|sum| sum <= input_limit) =>
                {
                    if self.admit_or_start_compaction(state, input_count.clone())? {
                        return Ok(());
                    }
                    break request;
                },
                Some(cap) => {
                    let next_cap = match cap_adjustments {
                        0 => cap
                            .saturating_sub(1)
                            .min(input_limit.saturating_sub(input_tokens)),
                        1 if cap > 1 => 1,
                        _ => 0,
                    };
                    if next_cap == 0 {
                        if self.admit_or_start_compaction(state, input_count.clone())? {
                            return Ok(());
                        }
                        return Err(failure(
                            BackendFailureKind::ContextExhausted,
                            format!(
                                "context_exhausted: {input_tokens} input tokens leave no positive request output cap within the {input_limit}-token input limit"
                            ),
                        ));
                    }
                    request_cap = Some(next_cap);
                    cap_adjustments += 1;
                },
                None if input_tokens < input_limit => {
                    if self.admit_or_start_compaction(state, input_count.clone())? {
                        return Ok(());
                    }
                    break request;
                },
                None => {
                    if self.admit_or_start_compaction(state, input_count.clone())? {
                        return Ok(());
                    }
                    return Err(failure(
                        BackendFailureKind::ContextExhausted,
                        format!(
                            "context_exhausted: {input_tokens} input tokens do not fit strictly below the {input_limit}-token input limit"
                        ),
                    ));
                },
            }
        };
        let cancellation = ModelConnectorCancellation::new();
        *self
            .shared_stop
            .response
            .lock()
            .map_err(|_| failure(BackendFailureKind::Cleanup, "native stop state is poisoned"))? =
            Some(cancellation.clone());
        let stream = match self.connector.start(request, cancellation) {
            Ok(stream) => stream,
            Err(error) => {
                *self.shared_stop.response.lock().map_err(|_| {
                    failure(BackendFailureKind::Cleanup, "native stop state is poisoned")
                })? = None;
                self.observe_connector_failure(state.turn, &error);
                return Err(map_connector_turn(error));
            },
        };
        state.stream = Some(stream);
        state.round += 1;
        state.response_id = None;
        state.assistant_activities.clear();
        state.reasoning_activities.clear();
        state.call_activities.clear();
        state.round_message_items.clear();
        state.round_messages.clear();
        state.round_refusals.clear();
        state.round_replay.clear();
        Ok(())
    }

    pub(super) fn admitted_request(
        &mut self,
        items: Vec<ModelConnectorInputItem>,
        tools: RequestToolExposure,
        session_id: SessionId,
    ) -> Result<(ModelConnectorRequest, InputCount), BackendFailure> {
        let input_limit = self.model_context.input_token_limit();
        let mut request_cap = self.model_context.max_output_tokens();
        let mut cap_adjustments = 0_u8;
        loop {
            let request = ModelConnectorRequest::new(
                items.clone(),
                tools.clone(),
                request_cap,
                self.config.reasoning_effort,
            )
            .map_err(map_connector_turn)?
            .with_cache_affinity_hint(ModelCacheAffinityHint::for_session(session_id));
            let input_count = self.count_request_input(&request)?;
            let input_tokens = input_count.planning_tokens();
            match request_cap {
                Some(cap)
                    if input_tokens
                        .checked_add(cap)
                        .is_some_and(|sum| sum <= input_limit) =>
                {
                    return Ok((request, input_count));
                },
                Some(cap) => {
                    let next_cap = match cap_adjustments {
                        0 => cap
                            .saturating_sub(1)
                            .min(input_limit.saturating_sub(input_tokens)),
                        1 if cap > 1 => 1,
                        _ => 0,
                    };
                    if next_cap == 0 {
                        return Err(failure(
                            BackendFailureKind::ContextExhausted,
                            "context_exhausted: summary request leaves no positive output cap",
                        ));
                    }
                    request_cap = Some(next_cap);
                    cap_adjustments += 1;
                },
                None if input_tokens < input_limit => return Ok((request, input_count)),
                None => {
                    return Err(failure(
                        BackendFailureKind::ContextExhausted,
                        "context_exhausted: summary request does not fit the input limit",
                    ));
                },
            }
        }
    }

    pub(super) fn count_input_for_items(
        &self,
        items: Vec<ModelConnectorInputItem>,
        tools: RequestToolExposure,
        session_id: SessionId,
    ) -> Result<InputCount, BackendFailure> {
        let request = ModelConnectorRequest::new(
            items,
            tools,
            self.model_context.max_output_tokens(),
            self.config.reasoning_effort,
        )
        .map_err(map_connector_turn)?
        .with_cache_affinity_hint(ModelCacheAffinityHint::for_session(session_id));
        self.count_request_input(&request)
    }
}
