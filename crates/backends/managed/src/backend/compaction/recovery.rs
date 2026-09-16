use yo_core::{BackendFailure, BackendFailureKind, BackendResumeTarget};

use super::super::{IdleCompactionState, NativeModelBackend, failure};

impl NativeModelBackend {
    pub(in crate::backend) fn restore_context_state(
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

    pub(in crate::backend) fn cleanup_idle_compaction(&mut self) {
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
