//! Internal failures translated at the CLI boundary.

use crate::wire::{ERROR_SCHEMA, FailureBody, FailureEnvelope};

#[derive(Debug)]
pub(crate) struct DiscoveryError {
    code: &'static str,
    message: String,
    retryable: bool,
    affected_ids: Vec<String>,
    affected_paths: Vec<String>,
    next_actions: Vec<String>,
}

impl DiscoveryError {
    pub(crate) fn request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(
            code,
            message,
            false,
            "correct the discovery request and retry",
        )
    }

    pub(crate) fn catalog(
        code: &'static str,
        message: impl Into<String>,
        affected_ids: Vec<String>,
        affected_paths: Vec<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
            affected_ids,
            affected_paths,
            next_actions: vec!["repair the working-tree catalog and retry".to_owned()],
        }
    }

    pub(crate) fn catalog_changed(affected_paths: Vec<String>) -> Self {
        Self {
            code: "catalog_changed_during_capture",
            message: "the working-tree catalog changed while it was being captured".to_owned(),
            retryable: true,
            affected_ids: Vec::new(),
            affected_paths,
            next_actions: vec!["retry after concurrent catalog edits finish".to_owned()],
        }
    }

    pub(crate) fn io(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(
            code,
            message,
            false,
            "check the path and permissions, then retry",
        )
    }

    fn new(
        code: &'static str,
        message: impl Into<String>,
        retryable: bool,
        next_action: &'static str,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            affected_ids: Vec::new(),
            affected_paths: Vec::new(),
            next_actions: vec![next_action.to_owned()],
        }
    }

    pub(crate) fn into_envelope(self) -> FailureEnvelope {
        FailureEnvelope {
            schema: ERROR_SCHEMA,
            ok: false,
            operation: "discover",
            error: FailureBody {
                code: self.code,
                message: self.message,
                retryable: self.retryable,
                affected_ids: self.affected_ids,
                affected_paths: self.affected_paths,
                next_actions: self.next_actions,
            },
        }
    }
}
