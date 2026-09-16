use yo_core::{
    ActivityKind, ActivityOutcome, BackendFailure, BackendFailureKind, ContextPressureDecision,
    ContextPressureObservation, ModelConnectorCancellation, ModelConnectorInputItem,
    ModelConnectorInputRole, ModelReplayItem, ModelReplayRole, RequestToolExposure,
};

use super::{
    super::{
        CompactionState, InputCount, NativeModelBackend, TurnState, context, failure,
        map_connector_turn,
    },
    projection::summary_source,
};

impl NativeModelBackend {
    pub(in crate::backend) fn admit_or_start_compaction(
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
            message_identity: None,
            message_done: false,
        });
        Ok(())
    }
}
