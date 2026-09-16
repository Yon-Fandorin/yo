use super::{
    AgentBackend, AgentEngine, AgentRuntime, RuntimeError, replacement::valid_replacement_binding,
};
use crate::{
    BackendFailureKind, BackendResumeTarget, ContinuationStrategy, ModelReplay,
    journal::SemanticRecord,
};

impl<B: AgentBackend> AgentRuntime<B> {
    pub(crate) fn initialize_resume(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<(), RuntimeError> {
        self.restore_resume_state(target)?;
        let evidence = self
            .backend
            .resume_session(target)
            .map_err(RuntimeError::backend)?;
        let expected = target.binding();
        if !expected.same_resume_identity(&evidence) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                BackendFailureKind::Session,
                "native resume returned a binding identity different from the durable Continuation Anchor",
            )));
        }
        self.publish_resume_snapshot()?;
        Ok(())
    }

    pub(crate) fn initialize_resume_replacing_binding(
        &mut self,
        target: &BackendResumeTarget,
    ) -> Result<(), RuntimeError> {
        self.restore_resume_state(target)?;
        if !matches!(
            target.binding().continuation_strategy(),
            ContinuationStrategy::ExactReplay { .. }
        ) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                BackendFailureKind::Unsupported,
                "binding replacement requires an exact-replay Continuation Anchor",
            )));
        }
        let evidence = self
            .backend
            .resume_session_replacing_binding(target)
            .map_err(RuntimeError::backend)?;
        let previous = target.binding();
        if !valid_replacement_binding(previous, &evidence, target.model_replay()) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                BackendFailureKind::Session,
                "exact-replay replacement returned an incompatible target binding",
            )));
        }
        self.publish_resume_snapshot()?;
        let epoch = target.epoch().checked_add(1).ok_or_else(|| {
            RuntimeError::backend(crate::BackendFailure::new(
                BackendFailureKind::Session,
                "backend binding epoch is exhausted",
            ))
        })?;
        let source = target.source().ok_or_else(|| {
            RuntimeError::backend(crate::BackendFailure::new(
                BackendFailureKind::Session,
                "exact-replay replacement requires one durable source",
            ))
        })?;
        if !self.journal.commit_exact_replay_replacement(
            target.epoch(),
            epoch,
            source,
            evidence.clone(),
        ) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                BackendFailureKind::Session,
                "exact-replay replacement could not publish its binding transition",
            )));
        }
        self.binding_epoch = Some(epoch);
        self.binding = Some(evidence.clone());
        self.continuation_strategy = Some(evidence.continuation_strategy());
        self.model_replay = target.model_replay().clone();
        if evidence.continuation_strategy() == ContinuationStrategy::BackendManagedState {
            self.model_replay = ModelReplay::default();
            self.replay_contract_rebind_required = false;
        } else {
            self.replay_contract_rebind_required = true;
        }
        self.resume_source = Some(source);
        self.binding_has_accepted_request = false;
        self.binding_has_unanchored_request = false;
        Ok(())
    }

    fn restore_resume_state(&mut self, target: &BackendResumeTarget) -> Result<(), RuntimeError> {
        let entries = self.journal.semantic_entries();
        self.engine =
            AgentEngine::from_journal(&entries, self.backend.capabilities().supports_steer())
                .map_err(|detail| {
                    RuntimeError::backend(crate::BackendFailure::new(
                        BackendFailureKind::Protocol,
                        detail,
                    ))
                })?;
        self.submission_ids = entries
            .iter()
            .filter_map(|entry| match entry.record() {
                SemanticRecord::CommandCommitted(committed) => committed.submission_id(),
                _ => None,
            })
            .collect();
        self.binding_epoch = Some(target.epoch());
        self.binding = Some(target.binding().clone());
        self.continuation_strategy = Some(target.binding().continuation_strategy());
        self.model_replay = target.model_replay().clone();
        self.replay_contract_rebind_required = target.replay_contract_rebind_required();
        self.input_image_history = target.input_image_history();
        self.resume_source = target.source();
        self.binding_has_accepted_request = target.binding_has_accepted_request();
        self.binding_has_unanchored_request = false;
        self.context_policy = target.context_policy().cloned();
        self.context_epoch = target.context_epoch();
        self.context_policy_initialized = true;
        self.context_replay_groups = target.model_replay_groups().to_vec();
        self.active_context_source = None;
        self.accepted_requests.clear();
        self.accepted_submissions.clear();
        Ok(())
    }

    fn publish_resume_snapshot(&mut self) -> Result<(), RuntimeError> {
        self.journal.initialize_durability();
        if !matches!(self.durability(), crate::JournalDurability::Durable { .. }) {
            return Err(RuntimeError::backend(crate::BackendFailure::new(
                BackendFailureKind::Session,
                "native resume could not publish its required complete Journal snapshot",
            )));
        }
        Ok(())
    }
}
