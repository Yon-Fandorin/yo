use yo_core::{
    BackendEvent, BackendFailure, BackendFailureKind, ContextCheckpointProposal,
    ModelConnectorEvent, ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorTerminal,
    ModelReplay, ModelReplayItem, ModelReplayRole, RequestToolExposure,
};

use super::{
    super::{
        CompactionState, NativeModelBackend, TurnState, context, failure, map_connector_cleanup,
        replay::replay_input,
    },
    projection::bind_summary_message,
};

impl NativeModelBackend {
    pub(in crate::backend) fn apply_compaction_response_event(
        &mut self,
        state: &mut TurnState,
        event: ModelConnectorEvent,
    ) -> Result<(), BackendFailure> {
        match event {
            ModelConnectorEvent::ResponseCreated { response_id } => {
                let Some(CompactionState::Summarizing {
                    response_id: current,
                    ..
                }) = state.compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary response arrived outside summary collection",
                    ));
                };
                if current.replace(response_id).is_some() {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary created more than one response identity",
                    ));
                }
            },
            ModelConnectorEvent::TextDelta {
                output_index,
                item_id,
                content_index,
                delta,
            } => {
                let Some(CompactionState::Summarizing {
                    body,
                    message_identity,
                    message_done,
                    ..
                }) = state.compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary text arrived outside summary collection",
                    ));
                };
                if content_index != 0
                    || *message_done
                    || !bind_summary_message(message_identity, output_index, &item_id)
                {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary must contain exactly one text message",
                    ));
                }
                body.push_str(&delta);
                if body.len() > 16 * 1024 * 1024 {
                    return Err(failure(
                        BackendFailureKind::ContextExhausted,
                        "context_exhausted: context summary exceeded its byte bound",
                    ));
                }
            },
            ModelConnectorEvent::MessageDone {
                output_index,
                item_id,
            } => {
                let Some(CompactionState::Summarizing {
                    message_identity,
                    message_done,
                    ..
                }) = state.compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary completion arrived outside summary collection",
                    ));
                };
                if *message_done || !bind_summary_message(message_identity, output_index, &item_id)
                {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary completed more than one message",
                    ));
                }
                *message_done = true;
            },
            ModelConnectorEvent::ReasoningDelta { .. }
            | ModelConnectorEvent::ProviderPrivateAssistant { .. } => {},
            ModelConnectorEvent::Terminal {
                response_id,
                status,
                usage,
            } => {
                let Some(CompactionState::Summarizing {
                    input_tokens_before,
                    summarized_groups,
                    retained_groups,
                    body,
                    response_id: created_response_id,
                    message_done,
                    ..
                }) = state.compaction.take()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary terminal arrived outside summary collection",
                    ));
                };
                if created_response_id.as_deref() != Some(response_id.as_str()) {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary terminal identity does not match its response",
                    ));
                }
                if !matches!(status, ModelConnectorTerminal::Completed) || !message_done {
                    return Err(failure(
                        BackendFailureKind::ContextExhausted,
                        "context_exhausted: context summary did not complete exactly once",
                    ));
                }
                context::validate_portable_summary(&body).map_err(|detail| {
                    failure(
                        BackendFailureKind::ContextExhausted,
                        format!("context_exhausted: {detail}"),
                    )
                })?;
                if let Some(mut stream) = state.stream.take() {
                    stream.shutdown().map_err(map_connector_cleanup)?;
                }
                *self.shared_stop.response.lock().map_err(|_| {
                    failure(BackendFailureKind::Cleanup, "native stop state is poisoned")
                })? = None;

                let summary_usage = self.context_summary_usage(
                    &response_id,
                    state.round.saturating_add(1),
                    &usage,
                )?;
                let mut checkpoint_items = vec![ModelReplayItem::Message {
                    role: ModelReplayRole::User,
                    content: body.clone(),
                    refusal: None,
                }];
                checkpoint_items.extend(retained_groups.iter().flatten().cloned());
                checkpoint_items.extend(state.delta.iter().cloned());
                let replay = ModelReplay::from_checkpoint(self.contract.clone(), checkpoint_items)
                    .map_err(|detail| failure(BackendFailureKind::ContextExhausted, detail))?;

                let mut successor_items = vec![ModelConnectorInputItem::Message {
                    role: ModelConnectorInputRole::System,
                    content: self.contract.system_prompt().to_owned(),
                    refusal: None,
                }];
                successor_items.extend(replay.items().iter().map(replay_input));
                let tool_exposure = if self.tool_exposure_enabled {
                    RequestToolExposure::enabled(self.registry.function_tools().map_err(
                        |error| failure(BackendFailureKind::Initialization, error.to_string()),
                    )?)
                } else {
                    RequestToolExposure::disabled()
                };
                let (_, input_tokens_after) =
                    self.admitted_request(successor_items, tool_exposure, state.turn.session_id())?;
                if !matches!(
                    context::admit_pressure(
                        &self.config.context_policy,
                        input_tokens_after.planning_tokens(),
                        self.model_context.input_token_limit(),
                        true,
                    ),
                    context::PressureAdmission::Admit { .. }
                ) {
                    return Err(failure(
                        BackendFailureKind::ContextExhausted,
                        "context_exhausted: compacted context still reaches the configured trigger",
                    ));
                }
                let proposal = ContextCheckpointProposal::new(
                    Some(state.turn),
                    self.config.context_policy.policy_revision(),
                    self.model_context.input_token_limit(),
                    input_tokens_before.planning_tokens(),
                    input_tokens_after.planning_tokens(),
                    self.contract.clone(),
                    body,
                    summarized_groups,
                    retained_groups,
                    state.delta.clone(),
                    summary_usage,
                )
                .map_err(|detail| failure(BackendFailureKind::ContextExhausted, detail))?;
                self.observe_model_request(state.turn, yo_core::ModelRequestOutcome::Succeeded);
                let proposal =
                    input_tokens_before.bind_checkpoint(proposal, &input_tokens_after)?;
                self.events
                    .push_back(BackendEvent::ContextCheckpointPrepared { proposal });
                state.compaction = Some(CompactionState::AwaitingCheckpoint { replay });
                state.compaction_attempted = true;
            },
            ModelConnectorEvent::RefusalDelta { .. }
            | ModelConnectorEvent::FunctionCallStarted { .. }
            | ModelConnectorEvent::FunctionArgumentsDelta { .. }
            | ModelConnectorEvent::FunctionCallDone { .. } => {
                return Err(failure(
                    BackendFailureKind::Protocol,
                    "context summary returned a forbidden semantic item",
                ));
            },
        }
        Ok(())
    }
}
