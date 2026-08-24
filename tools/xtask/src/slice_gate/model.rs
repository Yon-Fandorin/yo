use serde::{Deserialize, Serialize};

pub(super) const REQUEST_SCHEMA: &str = "yo.slice-gate-request/v1alpha1";
pub(super) const RESULT_SCHEMA: &str = "yo.slice-gate-result/v1alpha1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) schema: String,
    pub(super) candidate_commit: String,
    pub(super) required_lenses: Vec<String>,
    pub(super) validation_evidence: Vec<ValidationEvidence>,
    pub(super) review_evidence: Vec<ReviewEvidence>,
    #[serde(default)]
    pub(super) known_unverified_environments: Vec<String>,
    pub(super) risk: Risk,
    pub(super) approval: Option<Approval>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ValidationEvidence {
    pub(super) name: String,
    pub(super) argv: Vec<String>,
    pub(super) result_path: String,
    pub(super) result_hash: String,
    pub(super) candidate_commit: String,
    pub(super) reused: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReviewEvidence {
    pub(super) lens: String,
    pub(super) reviewer: String,
    pub(super) route: String,
    pub(super) verdict: String,
    pub(super) candidate_commit: String,
    pub(super) diff_hash: String,
    pub(super) result_path: String,
    pub(super) result_hash: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Risk {
    pub(super) classification: String,
    pub(super) rationale: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Approval {
    pub(super) kind: String,
    pub(super) authority: String,
    pub(super) scope: String,
    pub(super) candidate_commit: Option<String>,
    pub(super) diff_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ValidationSummary {
    pub(super) schema: String,
    pub(super) name: String,
    pub(super) status: String,
    pub(super) exit_code: i32,
    pub(super) elapsed_seconds: u64,
    pub(super) log_bytes: u64,
    pub(super) log_path: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ResultDocument {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) slice: String,
    pub(super) contract_id: String,
    pub(super) request_hash: String,
    pub(super) base_commit: String,
    pub(super) candidate_commit: String,
    pub(super) diff_hash: String,
    pub(super) changed_paths: Vec<String>,
    pub(super) required_lenses: Vec<String>,
    pub(super) validation: Vec<ValidationResult>,
    pub(super) review: Vec<ReviewResult>,
    pub(super) known_unverified_environments: Vec<String>,
    pub(super) risk: Risk,
    pub(super) approval: ApprovalResult,
    pub(super) commit_trailers: Vec<String>,
    pub(super) next_action: &'static str,
}

#[derive(Debug, Serialize)]
pub(super) struct ValidationResult {
    pub(super) name: String,
    pub(super) status: String,
    pub(super) reused: bool,
    pub(super) result_hash: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ReviewResult {
    pub(super) lens: String,
    pub(super) reviewer: String,
    pub(super) route: String,
    pub(super) verdict: String,
    pub(super) result_hash: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ApprovalResult {
    pub(super) required: bool,
    pub(super) satisfied: bool,
    pub(super) kind: Option<String>,
    pub(super) authority: Option<String>,
    pub(super) scope: Option<String>,
}
