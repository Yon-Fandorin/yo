use yo_core::{
    BackendEvent, BackendFailure, BackendFailureKind, ContextCheckpointProposal,
    ModelConnectorEvent, ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorTerminal,
    ModelReplay, ModelReplayItem, ModelReplayRole, RequestToolExposure,
};

use super::{
    super::{
        IdleCompactionState, NativeModelBackend, context, failure, map_connector_cleanup,
        replay::replay_input,
    },
    projection::bind_summary_message,
};

impl NativeModelBackend {
    pub(in crate::backend) fn apply_idle_compaction_event(
        &mut self,
        event: ModelConnectorEvent,
    ) -> Result<(), BackendFailure> {
        match event {
            ModelConnectorEvent::ResponseCreated { response_id } => {
                let Some(IdleCompactionState::Summarizing {
                    response_id: current,
                    ..
                }) = self.idle_compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary response arrived outside summary collection",
                    ));
                };
                if current.replace(response_id).is_some() {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary created more than one response identity",
                    ));
                }
            },
            ModelConnectorEvent::TextDelta {
                output_index,
                item_id,
                content_index,
                delta,
            } => {
                let Some(IdleCompactionState::Summarizing {
                    body,
                    message_identity,
                    message_done,
                    ..
                }) = self.idle_compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary text arrived outside summary collection",
                    ));
                };
                if content_index != 0
                    || *message_done
                    || !bind_summary_message(message_identity, output_index, &item_id)
                {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary must contain exactly one text message",
                    ));
                }
                body.push_str(&delta);
                if body.len() > 16 * 1024 * 1024 {
                    return Err(failure(
                        BackendFailureKind::ContextExhausted,
                        "context_exhausted: idle context summary exceeded its byte bound",
                    ));
                }
            },
            ModelConnectorEvent::MessageDone {
                output_index,
                item_id,
            } => {
                let Some(IdleCompactionState::Summarizing {
                    message_identity,
                    message_done,
                    ..
                }) = self.idle_compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary completion arrived outside summary collection",
                    ));
                };
                if *message_done || !bind_summary_message(message_identity, output_index, &item_id)
                {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary completed more than one message",
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
                let Some(IdleCompactionState::Summarizing {
                    input_tokens_before,
                    summarized_groups,
                    retained_groups,
                    body,
                    response_id: created_response_id,
                    message_done,
                    mut stream,
                    ..
                }) = self.idle_compaction.take()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary terminal arrived outside summary collection",
                    ));
                };
                stream.shutdown().map_err(map_connector_cleanup)?;
                *self.shared_stop.response.lock().map_err(|_| {
                    failure(BackendFailureKind::Cleanup, "native stop state is poisoned")
                })? = None;
                if created_response_id.as_deref() != Some(response_id.as_str())
                    || !matches!(status, ModelConnectorTerminal::Completed)
                    || !message_done
                {
                    return Err(failure(
                        BackendFailureKind::ContextExhausted,
                        "context_exhausted: idle context summary did not complete exactly once",
                    ));
                }
                let summary_usage = self.context_summary_usage(&response_id, 1, &usage)?;
                context::validate_portable_summary(&body).map_err(|_| {
                    failure(
                        BackendFailureKind::CommandRejected,
                        "Context compaction did not match the required summary format; the original context was preserved.",
                    )
                })?;
                let mut checkpoint_items = vec![ModelReplayItem::Message {
                    role: ModelReplayRole::User,
                    content: body.clone(),
                    refusal: None,
                }];
                checkpoint_items.extend(retained_groups.iter().flatten().cloned());
                let replay = ModelReplay::from_checkpoint(self.contract.clone(), checkpoint_items)
                    .map_err(|detail| failure(BackendFailureKind::ContextExhausted, detail))?;
                let tool_exposure = if self.tool_exposure_enabled {
                    RequestToolExposure::enabled(self.registry.function_tools().map_err(
                        |error| failure(BackendFailureKind::Initialization, error.to_string()),
                    )?)
                } else {
                    RequestToolExposure::disabled()
                };
                let mut successor_items = vec![ModelConnectorInputItem::Message {
                    role: ModelConnectorInputRole::System,
                    content: self.contract.system_prompt().to_owned(),
                    refusal: None,
                }];
                successor_items.extend(replay.items().iter().map(replay_input));
                let session_id = self.session.expect("idle compaction has an open Session");
                let (_, input_tokens_after) =
                    self.admitted_request(successor_items, tool_exposure, session_id)?;
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
                        "context_exhausted: idle compacted context did not admit the payload",
                    ));
                }
                if input_tokens_after.planning_tokens() >= input_tokens_before.planning_tokens() {
                    return Err(failure(
                        BackendFailureKind::CommandRejected,
                        "Context compaction did not reduce the input size; the original context was preserved.",
                    ));
                }
                let proposal = ContextCheckpointProposal::new(
                    None,
                    self.config.context_policy.policy_revision(),
                    self.model_context.input_token_limit(),
                    input_tokens_before.planning_tokens(),
                    input_tokens_after.planning_tokens(),
                    self.contract.clone(),
                    body,
                    summarized_groups,
                    retained_groups,
                    Vec::new(),
                    summary_usage,
                )
                .map_err(|detail| failure(BackendFailureKind::ContextExhausted, detail))?;
                if let Some(observer) = self.request_observer.as_mut() {
                    let _ = observer.observe(yo_core::ModelRequestOutcome::Succeeded);
                }
                let proposal =
                    input_tokens_before.bind_checkpoint(proposal, &input_tokens_after)?;
                self.events
                    .push_back(BackendEvent::ContextCheckpointPrepared { proposal });
                self.idle_compaction = Some(IdleCompactionState::AwaitingCheckpoint { replay });
            },
            ModelConnectorEvent::RefusalDelta { .. }
            | ModelConnectorEvent::FunctionCallStarted { .. }
            | ModelConnectorEvent::FunctionArgumentsDelta { .. }
            | ModelConnectorEvent::FunctionCallDone { .. } => {
                return Err(failure(
                    BackendFailureKind::ContextExhausted,
                    "context_exhausted: idle context summary returned a forbidden semantic item",
                ));
            },
        }
        Ok(())
    }
}
