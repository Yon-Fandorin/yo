use serde::Deserialize;

use super::model::{CheckpointIdentity, ContextResult};
use crate::review_protocol::{Captured, NamedCaptured};

mod activation;
mod context;
mod evidence;
mod prospective;

pub(super) use activation::parse_activation_request;
#[cfg(test)]
pub(super) use context::capture_context_from_result;
pub(super) use context::{capture_context, capture_context_request, capture_context_with_request};
pub(super) use evidence::{
    capture_authorities, capture_diff, capture_validation, captured, require_hash,
    require_repository_path, same_capture, same_captures, same_named_captures,
};
#[cfg(test)]
pub(super) use prospective::capture_prospective_context_from_result;
pub(super) use prospective::capture_prospective_context_with_request;

pub(super) struct ContextCapture {
    pub(super) result: ContextResult,
    pub(super) request: Captured,
    pub(super) context: Captured,
    pub(super) manifest: Captured,
    pub(super) active_checkpoint: CheckpointIdentity,
    pub(super) included_ids: Vec<String>,
}

pub(super) struct ProspectiveCapture {
    pub(super) activation_request: Captured,
    pub(super) proposed_checkpoint: Captured,
    pub(super) proposed_active_record: Captured,
    pub(super) predecessor_active_record_hash: Option<String>,
}

pub(super) struct Inputs {
    pub(super) base_commit: String,
    pub(super) candidate_commit: String,
    pub(super) diff: Captured,
    pub(super) context: ContextCapture,
    pub(super) prospective: Option<ProspectiveCapture>,
    pub(super) authorities: Vec<Captured>,
    pub(super) slice_contract: Captured,
    pub(super) validation: Vec<NamedCaptured>,
    pub(super) lenses: Vec<String>,
    pub(super) questions: Vec<String>,
    pub(super) required_knowledge_ids: Vec<String>,
    pub(super) delivery_profile_bytes: Vec<u8>,
    pub(super) max_tokens: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActivationRequest {
    pub(super) schema: String,
    pub(super) checkpoint_id: String,
    pub(super) checkpoint_hash: String,
    pub(super) replace_active_hash: Option<String>,
}
