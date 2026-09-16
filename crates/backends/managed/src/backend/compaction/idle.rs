use yo_core::{
    BackendFailure, BackendFailureKind, ContextStrategy, ModelConnectorCancellation,
    ModelConnectorInputItem, ModelConnectorInputRole, RequestToolExposure,
};

use super::{
    super::{IdleCompactionState, NativeModelBackend, context, failure, replay::replay_input},
    projection::summary_source,
};

impl NativeModelBackend {
    pub(in crate::backend) fn start_idle_compaction(
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
            message_identity: None,
            message_done: false,
            stream,
        });
        Ok(())
    }
}
