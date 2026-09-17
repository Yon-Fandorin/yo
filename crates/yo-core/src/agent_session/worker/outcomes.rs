use std::sync::PoisonError;

use super::{super::AgentSessionError, AgentWorker, ChangeLane, WorkerExit};
use crate::{
    AgentRejection, BackendFailureKind, RuntimeError, SubmissionOutcome, SubmissionRejection,
    SubmissionRejectionKind,
};

impl AgentWorker {
    pub(super) fn record_submission_outcome(&self, outcome: SubmissionOutcome) {
        self.submission_outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(outcome);
    }

    pub(super) fn finish_after_failure(
        &mut self,
        primary: AgentSessionError,
        changes: &mut ChangeLane,
    ) -> WorkerExit {
        let primary_changed = matches!(
            &primary,
            AgentSessionError::Runtime(error) if !error.terminal_events().is_empty()
        );
        let cleanup = self.runtime.shutdown();
        let cleanup_changed = match &cleanup {
            Ok(events) => !events.is_empty(),
            Err(error) => !error.terminal_events().is_empty(),
        };
        if primary_changed || cleanup_changed {
            let _ = changes.changed();
        }
        let _ = changes.failure(primary);
        WorkerExit::from_cleanup(cleanup)
    }
}

pub(in crate::agent_session) fn submission_rejection(
    error: &RuntimeError,
) -> Option<SubmissionRejection> {
    let (kind, detail) = match error {
        RuntimeError::InputRejected(rejection) => return Some(rejection.clone()),
        RuntimeError::CommandRejected(rejection) => (
            match rejection {
                AgentRejection::TurnNotActive { .. } | AgentRejection::SessionMismatch { .. } => {
                    SubmissionRejectionKind::StaleReference
                },
                _ => SubmissionRejectionKind::Incompatible,
            },
            rejection.to_string(),
        ),
        RuntimeError::Backend {
            failure,
            terminal_events,
        } if failure.kind() == BackendFailureKind::CommandRejected
            && terminal_events.is_empty() =>
        {
            (
                SubmissionRejectionKind::Incompatible,
                failure.message().to_owned(),
            )
        },
        _ => return None,
    };
    Some(SubmissionRejection::new(kind, detail))
}

pub(in crate::agent_session) fn context_compaction_rejection(
    error: &AgentSessionError,
) -> Option<String> {
    match error {
        AgentSessionError::Runtime(RuntimeError::CommandRejected(rejection)) => {
            Some(rejection.to_string())
        },
        AgentSessionError::Runtime(RuntimeError::Backend {
            failure,
            terminal_events,
        }) if failure.kind() == BackendFailureKind::CommandRejected
            && terminal_events.is_empty() =>
        {
            Some(failure.message().to_owned())
        },
        AgentSessionError::ContextCompactionRequiresIdle => Some(error.to_string()),
        _ => None,
    }
}
