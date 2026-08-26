use serde::{Deserialize, Serialize};

use super::super::{ResultDocument, Risk};

pub(super) const REQUEST_SCHEMA: &str = "yo.slice-gate-prepare-request/v1";
pub(super) const REQUEST_SCHEMA_V1_ALPHA1: &str = "yo.slice-gate-prepare-request/v1alpha1";
pub(super) const RESULT_SCHEMA: &str = "yo.slice-gate-prepare-result/v1";
pub(super) const RESULT_SCHEMA_V1_ALPHA1: &str = "yo.slice-gate-prepare-result/v1alpha1";
pub(super) const REVIEW_CARRY_SCHEMA: &str = "yo.canonical-approval-review-carry/v1alpha1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrepareRequest {
    pub(super) schema: String,
    pub(super) manifest_path: String,
    pub(super) validation_commands: Vec<ValidationCommand>,
    #[serde(default)]
    pub(super) review_runs: Vec<ReviewRun>,
    #[serde(default)]
    pub(super) known_unverified_environments: Vec<String>,
    pub(super) risk: Risk,
    pub(super) approval: Option<CompactApproval>,
    #[serde(default)]
    pub(super) review_carry: Option<CanonicalApprovalReviewCarry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CanonicalApprovalReviewCarry {
    pub(super) schema: String,
    pub(super) knowledge_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ValidationCommand {
    pub(super) name: String,
    pub(super) argv: Vec<String>,
    pub(super) reused: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReviewRun {
    pub(super) source: ReviewSource,
    pub(super) result_path: String,
    pub(super) verdicts: Vec<LensVerdict>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ReviewSource {
    DeliveryReceipt { receipt_path: String, class: String },
    DeclaredRoute { route: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LensVerdict {
    pub(super) lens: String,
    pub(super) verdict: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompactApproval {
    pub(super) kind: String,
    pub(super) authority: String,
    pub(super) scope: String,
}

#[derive(Debug, Serialize)]
pub(super) struct PrepareResult {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) request_path: String,
    pub(super) request_hash: String,
    pub(super) review_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) review_carry: Option<CanonicalApprovalReviewCarryResult>,
    pub(super) gate: ResultDocument,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct CanonicalApprovalReviewCarryResult {
    pub(super) schema: &'static str,
    pub(super) knowledge_id: String,
    pub(super) reviewed_candidate: String,
    pub(super) candidate_commit: String,
    pub(super) knowledge_path: String,
    pub(super) approval_path: String,
    pub(super) revision: String,
    pub(super) reviewer: String,
    pub(super) reviewed_at: String,
    pub(super) request_hash: String,
    pub(super) approval_hash: String,
    pub(super) replaced_revision: Option<String>,
    pub(super) transition_diff_hash: String,
}
