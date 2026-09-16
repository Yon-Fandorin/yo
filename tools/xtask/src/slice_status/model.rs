use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::delivery;
use crate::slice_contract;

pub(super) const RESULT_SCHEMA: &str = "yo.slice-status/v1alpha4";
pub(super) const JSON_LIMIT: usize = 8 * 1024 * 1024;
pub(super) const MAX_JSON_FILES: usize = 256;
pub(super) const MAX_SCAN_DEPTH: usize = 6;

#[derive(Clone, Debug)]
pub(crate) struct SliceState {
    pub(crate) worktree: PathBuf,
    pub(crate) branch: String,
    pub(crate) head: String,
    pub(crate) bound: slice_contract::BoundSlice,
    pub(crate) clean: bool,
}

pub(super) struct Artifacts {
    pub(super) validations: Vec<ValidationSummary>,
    pub(super) gate_requests: usize,
    pub(super) gate_request: Option<PathBuf>,
    pub(super) claims: usize,
    pub(super) delivery_receipts: usize,
    pub(super) review_rounds: usize,
    pub(super) durable_requests: u64,
    pub(super) prior_findings: usize,
    pub(super) superseded: usize,
    pub(super) delivery: delivery::Projection,
    pub(super) delivery_request: Option<PathBuf>,
}

impl Default for Artifacts {
    fn default() -> Self {
        Self {
            validations: Vec::new(),
            gate_requests: 0,
            gate_request: None,
            claims: 0,
            delivery_receipts: 0,
            review_rounds: 0,
            durable_requests: 0,
            prior_findings: 0,
            superseded: 0,
            delivery: delivery::prepared(),
            delivery_request: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ValidationSummary {
    pub(super) name: String,
    pub(super) status: String,
    pub(super) log_hash: String,
    pub(super) path: String,
    pub(super) reused: bool,
}

pub(super) struct ReviewLineage {
    pub(super) packets: usize,
    pub(super) latest_candidate: Option<String>,
    pub(super) status: &'static str,
    pub(super) current_review_ids: BTreeSet<String>,
    pub(super) latest_review_ids: BTreeSet<String>,
    pub(super) current_validations: Vec<EffectiveValidation>,
}

pub(super) struct EffectiveValidation {
    pub(super) name: String,
    pub(super) path: PathBuf,
    pub(super) hash: String,
    pub(super) reused: bool,
}

#[derive(Default)]
pub(super) struct ScanBudget {
    pub(super) json_files: usize,
}

pub(super) struct CoordinationScope<'a> {
    pub(super) repository: &'a Path,
    pub(super) workspace: &'a Path,
    pub(super) candidate: &'a str,
    pub(super) current_review_ids: &'a BTreeSet<String>,
    pub(super) latest_review_ids: &'a BTreeSet<String>,
    pub(super) current_validations: &'a [EffectiveValidation],
}

#[derive(Serialize)]
pub(super) struct ResultDocument {
    pub(super) schema: &'static str,
    pub(super) ok: bool,
    pub(super) slice: String,
    pub(super) branch: String,
    pub(super) base_commit: String,
    pub(super) candidate_commit: String,
    pub(super) clean: bool,
    pub(super) review_lineage: &'static str,
    pub(super) review_packets: usize,
    pub(super) review_rounds: usize,
    pub(super) review_chain: Vec<String>,
    pub(super) latest_packet_candidate: Option<String>,
    pub(super) validation_summaries: Vec<ValidationSummary>,
    pub(super) gate_requests: usize,
    pub(super) delivery_claims: usize,
    pub(super) delivery_receipts: usize,
    pub(super) durable_external_requests: u64,
    pub(super) superseded_artifacts: usize,
    pub(super) delivery: delivery::Projection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) blocking_reason: Option<String>,
    pub(super) next_action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_argv: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_working_directory: Option<String>,
}
