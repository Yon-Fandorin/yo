use serde::{Deserialize, Serialize};

use super::super::{ResultDocument, Risk};

pub(super) const REQUEST_SCHEMA: &str = "yo.slice-gate-prepare-request/v1";
pub(super) const RESULT_SCHEMA: &str = "yo.slice-gate-prepare-result/v1";

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
    pub(super) gate: ResultDocument,
}
