//! Active and idle context-summary lifecycle and checkpoint proposals.

use serde_json::json;
use yo_core::{
    ActivityKind, ActivityOutcome, BackendEvent, BackendFailure, BackendFailureKind,
    BackendResumeTarget, CacheReadInputTokens, ContextCheckpointProposal, ContextPressureDecision,
    ContextPressureObservation, ContextStrategy, ImageSummarySource, ModelConnectorCancellation,
    ModelConnectorEvent, ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorTerminal,
    ModelReplay, ModelReplayItem, ModelReplayRole, RequestToolExposure,
};

use super::{
    CompactionState, IdleCompactionState, InputCount, NativeModelBackend, TurnState, context,
    failure, map_connector_cleanup, map_connector_turn, replay::replay_input,
};

impl NativeModelBackend {
    pub(super) fn admit_or_start_compaction(
        &mut self,
        state: &mut TurnState,
        input_count: InputCount,
    ) -> Result<bool, BackendFailure> {
        use context::PressureAdmission;

        if !self.context_policy_active {
            return Ok(false);
        }

        let input_tokens = input_count.planning_tokens();
        let decision = context::admit_pressure(
            &self.config.context_policy,
            input_tokens,
            self.model_context.input_token_limit(),
            state.compaction_attempted,
        );
        let warning = match decision {
            PressureAdmission::Admit { warning }
            | PressureAdmission::Compact { warning }
            | PressureAdmission::Reject { warning } => warning,
        };
        if warning {
            let mut observation = ContextPressureObservation::new(
                input_tokens,
                self.model_context.input_token_limit(),
                self.config.context_policy.warning_percent(),
                self.config.context_policy.trigger_percent(),
                match decision {
                    PressureAdmission::Admit { .. } => ContextPressureDecision::Admit,
                    PressureAdmission::Compact { .. } => ContextPressureDecision::Compact,
                    PressureAdmission::Reject { .. } => ContextPressureDecision::Reject,
                },
            )
            .expect("an admitted context policy produces a valid pressure observation");
            if let Some(accounting) = input_count.accounting() {
                observation = observation
                    .with_accounting(accounting.clone())
                    .map_err(|detail| failure(BackendFailureKind::Protocol, detail))?;
            }
            let activity = self.next_activity(state.turn)?;
            self.queue_activity_text(
                activity,
                ActivityKind::ModelWork,
                observation.to_snapshot_json(),
                Some(ActivityOutcome::Completed),
            );
        }
        match decision {
            PressureAdmission::Admit { .. } => Ok(false),
            PressureAdmission::Compact { .. } => {
                self.start_compaction_summary(state, input_count)?;
                Ok(true)
            },
            PressureAdmission::Reject { .. } => Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: context pressure remained above the configured trigger",
            )),
        }
    }

    fn start_compaction_summary(
        &mut self,
        state: &mut TurnState,
        input_tokens_before: InputCount,
    ) -> Result<(), BackendFailure> {
        let starts_with_current_input = matches!(
            state.delta.first(),
            Some(
                ModelReplayItem::Message {
                    role: ModelReplayRole::User,
                    refusal: None,
                    ..
                } | ModelReplayItem::MultimodalUser { .. }
            )
        );
        let completed_tool_boundary = state.round > 0
            && state
                .delta
                .iter()
                .any(|item| matches!(item, ModelReplayItem::FunctionCall { .. }))
            && state
                .delta
                .iter()
                .any(|item| matches!(item, ModelReplayItem::FunctionCallOutput { .. }))
            && state.pending_calls.is_empty()
            && state.active_tool.is_none()
            && state.ready_tool.is_none()
            && state.dispatch_tool.is_none()
            && state.awaiting_approval.is_none();
        if !starts_with_current_input
            || (state.round == 0 && state.delta.len() != 1)
            || (state.round > 0 && !completed_tool_boundary)
        {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: active-Turn compaction requires a completely admitted current suffix",
            ));
        }
        let summarized_count = if state.round == 0 {
            self.replay_groups.len().saturating_sub(1)
        } else {
            self.replay_groups.len()
        };
        if state.compaction.is_some() || summarized_count == 0 {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: no older complete semantic prefix is available to compact",
            ));
        }
        let summarized_groups = self.replay_groups[..summarized_count].to_vec();
        let retained_groups = self.replay_groups[summarized_count..].to_vec();
        let Some(visible_prefix) = summary_source(&summarized_groups)? else {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: the compactable prefix has no visible semantic history",
            ));
        };
        let mut items = vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::System,
            content: context::PORTABLE_SUMMARY_INSTRUCTION.to_owned(),
            refusal: None,
        }];
        items.push(visible_prefix);
        let (request, _) = self.admitted_request(
            items,
            RequestToolExposure::disabled(),
            state.turn.session_id(),
        )?;
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
        state.compaction = Some(CompactionState::Summarizing {
            input_tokens_before,
            summarized_groups,
            retained_groups,
            body: String::new(),
            response_id: None,
            message_done: false,
        });
        Ok(())
    }

    pub(super) fn start_idle_compaction(
        &mut self,
        guidance: Option<String>,
    ) -> Result<(), BackendFailure> {
        if self.context_exhausted {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: this binding cannot attempt another context compaction",
            ));
        }
        let session_id = self.session.ok_or_else(|| {
            failure(
                BackendFailureKind::Session,
                "context compaction requires an open Session",
            )
        })?;
        if self.turn.is_some() || self.idle_compaction.is_some() {
            return Err(failure(
                BackendFailureKind::Turn,
                "context compaction requires an idle Session",
            ));
        }
        if !self.context_policy_active
            || !self.config.context_policy.enabled()
            || self.config.context_policy.strategy() != ContextStrategy::PortableSummaryV1Alpha1
        {
            return Err(failure(
                BackendFailureKind::CommandRejected,
                "the current context policy does not permit manual compaction",
            ));
        }
        if self.replay_groups.len() < 2 {
            return Err(failure(
                BackendFailureKind::CommandRejected,
                "no older complete semantic prefix is available to compact",
            ));
        }
        let tool_exposure =
            if self.tool_exposure_enabled {
                RequestToolExposure::enabled(self.registry.function_tools().map_err(|error| {
                    failure(BackendFailureKind::Initialization, error.to_string())
                })?)
            } else {
                RequestToolExposure::disabled()
            };
        let mut current_items = vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::System,
            content: self.contract.system_prompt().to_owned(),
            refusal: None,
        }];
        current_items.extend(self.replay.items().iter().map(replay_input));
        let input_tokens_before =
            self.count_input_for_items(current_items, tool_exposure, session_id)?;

        let retained_groups = vec![
            self.replay_groups
                .last()
                .expect("two replay groups have a newest group")
                .clone(),
        ];
        let summarized_groups = self.replay_groups[..self.replay_groups.len() - 1].to_vec();
        let Some(visible_prefix) = summary_source(&summarized_groups)? else {
            return Err(failure(
                BackendFailureKind::CommandRejected,
                "the compactable prefix has no visible semantic history",
            ));
        };
        let mut instruction = context::PORTABLE_SUMMARY_INSTRUCTION.to_owned();
        if let Some(guidance) = guidance {
            instruction.push_str("\n\nUser guidance for this checkpoint:\n");
            instruction.push_str(&guidance);
        }
        let mut summary_items = vec![ModelConnectorInputItem::Message {
            role: ModelConnectorInputRole::System,
            content: instruction,
            refusal: None,
        }];
        summary_items.push(visible_prefix);
        let (request, _) =
            self.admitted_request(summary_items, RequestToolExposure::disabled(), session_id)?;
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
                return Err(failure(
                    BackendFailureKind::ContextExhausted,
                    format!("context_exhausted: context summary request failed: {error}"),
                ));
            },
        };
        self.idle_compaction = Some(IdleCompactionState::Summarizing {
            input_tokens_before,
            summarized_groups,
            retained_groups,
            body: String::new(),
            response_id: None,
            message_done: false,
            stream,
        });
        Ok(())
    }

    pub(super) fn apply_compaction_response_event(
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
                content_index,
                delta,
                ..
            } => {
                let Some(CompactionState::Summarizing {
                    body, message_done, ..
                }) = state.compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary text arrived outside summary collection",
                    ));
                };
                if output_index != 0 || content_index != 0 || *message_done {
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
            ModelConnectorEvent::MessageDone { output_index, .. } => {
                let Some(CompactionState::Summarizing { message_done, .. }) =
                    state.compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary completion arrived outside summary collection",
                    ));
                };
                if output_index != 0 || std::mem::replace(message_done, true) {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "context summary completed more than one message",
                    ));
                }
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

    fn context_summary_usage(
        &self,
        response_id: &str,
        round: usize,
        usage: &yo_core::ModelConnectorUsage,
    ) -> Result<serde_json::Value, BackendFailure> {
        let (Some(input_tokens), Some(output_tokens), Some(total_tokens)) =
            (usage.input_tokens, usage.output_tokens, usage.total_tokens)
        else {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: context summary usage is incomplete",
            ));
        };
        let cache_read_input_tokens = match &usage.cache_read_input_tokens {
            CacheReadInputTokens::Reported {
                tokens,
                source_profile,
            } => json!({
                "availability": "reported",
                "tokens": tokens,
                "source_profile": source_profile.as_str(),
            }),
            CacheReadInputTokens::Absent { source_profile } => json!({
                "availability": "absent",
                "source_profile": source_profile.as_str(),
            }),
            CacheReadInputTokens::Unsupported => json!({
                "availability": "unsupported",
            }),
        };
        Ok(json!({
            "schema": "yo.model-usage-receipt/v1",
            "response_id": response_id,
            "round": u64::try_from(round).unwrap_or(u64::MAX),
            "provider": self.binding.provider_id().as_str(),
            "account": self.binding.account_id().as_str(),
            "model": self.binding.model_id().as_str(),
            "connector": self.binding.connector_id().as_str(),
            "api_dialect": self.binding.api_dialect().as_str(),
            "base_url": self.binding.endpoint().as_str(),
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "total_tokens": total_tokens,
                "reasoning_tokens": usage.reasoning_tokens,
            },
            "cache_read_input_tokens": cache_read_input_tokens,
        }))
    }

    pub(super) fn apply_idle_compaction_event(
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
                content_index,
                delta,
                ..
            } => {
                let Some(IdleCompactionState::Summarizing {
                    body, message_done, ..
                }) = self.idle_compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary text arrived outside summary collection",
                    ));
                };
                if output_index != 0 || content_index != 0 || *message_done {
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
            ModelConnectorEvent::MessageDone { output_index, .. } => {
                let Some(IdleCompactionState::Summarizing { message_done, .. }) =
                    self.idle_compaction.as_mut()
                else {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary completion arrived outside summary collection",
                    ));
                };
                if output_index != 0 || std::mem::replace(message_done, true) {
                    return Err(failure(
                        BackendFailureKind::Protocol,
                        "idle context summary completed more than one message",
                    ));
                }
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

    pub(super) fn restore_context_state(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<(), BackendFailure> {
        let legacy_groups = target.context_policy().is_none()
            && target.model_replay_groups().is_empty()
            && !target.model_replay().items().is_empty();
        if target.context_policy().is_some() != target.context_epoch().is_some()
            || target.model_replay_groups().iter().any(Vec::is_empty)
            || (!legacy_groups
                && !target
                    .model_replay_groups()
                    .iter()
                    .flatten()
                    .eq(target.model_replay().items()))
        {
            return Err(failure(
                BackendFailureKind::Session,
                "durable context policy, epoch, or replay groups are internally inconsistent",
            ));
        }
        if let Some(policy) = target.context_policy() {
            self.config.context_policy = policy.clone();
        }
        self.context_policy_active = target.context_policy().is_some();
        self.replay_groups = if legacy_groups {
            vec![target.model_replay().items().to_vec()]
        } else {
            target.model_replay_groups().to_vec()
        };
        Ok(())
    }

    pub(super) fn cleanup_idle_compaction(&mut self) {
        if let Some(IdleCompactionState::Summarizing { mut stream, .. }) =
            self.idle_compaction.take()
        {
            stream.cancel();
            let _ = stream.shutdown();
        } else {
            self.idle_compaction = None;
        }
        if let Ok(mut response) = self.shared_stop.response.lock() {
            *response = None;
        }
    }
}

// Summary source is inert data, not provider-native assistant/tool replay. In particular,
// private assistant envelopes must never be required or forwarded by this projection.
fn summary_source(
    groups: &[Vec<ModelReplayItem>],
) -> Result<Option<ModelConnectorInputItem>, BackendFailure> {
    if groups
        .iter()
        .flatten()
        .any(|item| matches!(item, ModelReplayItem::MultimodalUser { .. }))
    {
        return ImageSummarySource::from_replay_groups(groups)
            .map(|source| Some(ModelConnectorInputItem::ImageSummarySource { source }))
            .map_err(|detail| failure(BackendFailureKind::ContextExhausted, detail));
    }
    const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
    let mut content = String::from("{\"history\":[");
    let mut count = 0;
    for item in groups.iter().flatten() {
        let record = match item {
            ModelReplayItem::Message {
                role,
                content,
                refusal,
            } => {
                let role = match role {
                    ModelReplayRole::System => "system",
                    ModelReplayRole::Developer => "developer",
                    ModelReplayRole::User => "user",
                    ModelReplayRole::Assistant => "assistant",
                };
                json!({"type":"message", "role":role, "content":content, "refusal":refusal})
            },
            ModelReplayItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                json!({"type":"function_call", "call_id":call_id, "name":name, "arguments":arguments})
            },
            ModelReplayItem::FunctionCallOutput { call_id, output } => {
                json!({"type":"function_call_output", "call_id":call_id, "output":output})
            },
            ModelReplayItem::ProviderPrivateAssistant { .. } => continue,
            ModelReplayItem::MultimodalUser { .. } => {
                unreachable!("image summaries use the typed source")
            },
        };
        let encoded = record.to_string();
        if content
            .len()
            .saturating_add(encoded.len())
            .saturating_add(2 + usize::from(count > 0))
            > MAX_SOURCE_BYTES
        {
            return Err(failure(
                BackendFailureKind::ContextExhausted,
                "context_exhausted: encoded visible summary source exceeds the 16-MiB message limit",
            ));
        }
        if count > 0 {
            content.push(',');
        }
        content.push_str(&encoded);
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }
    content.push_str("]}");
    Ok(Some(ModelConnectorInputItem::Message {
        role: ModelConnectorInputRole::User,
        content,
        refusal: None,
    }))
}
