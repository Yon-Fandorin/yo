use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedResult {
    pub(crate) verdicts: Vec<Verdict>,
    pub(crate) findings: Vec<Finding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct InspectedResult {
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) verified: VerifiedResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Verdict {
    pub(crate) lens: String,
    pub(crate) verdict: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Finding {
    pub(crate) finding_id: String,
    pub(crate) summary: String,
    pub(crate) lenses: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResultDocument {
    pub(super) schema: String,
    pub(super) review_id: String,
    pub(super) candidate_commit: String,
    pub(super) verdicts: Vec<Verdict>,
    pub(super) findings: Vec<Finding>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CorrectionRequest {
    pub(super) schema: String,
    pub(super) manifest_path: String,
    pub(super) manifest_hash: String,
    pub(super) delivery_receipt_path: String,
    pub(super) delivery_receipt_hash: String,
    pub(super) review_result_path: String,
    pub(super) review_result_hash: String,
}

#[derive(Debug, Serialize)]
pub(super) struct CorrectionResult {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) status: &'static str,
    pub(super) next_action: &'static str,
    pub(super) provider_requests: usize,
    pub(super) expected_review_id: String,
    pub(super) observed_review_id: String,
    pub(super) expected_candidate_commit: String,
    pub(super) observed_candidate_commit: String,
    pub(super) session_id: String,
    pub(super) route: String,
    pub(super) immutable_result_hash: String,
}
