use std::path::Path;

use serde::Serialize;

use crate::model::{KnowledgeUnit, Owner, Source};

const CHECK_SCHEMA: &str = "methexis.check/v1alpha1";
pub(crate) mod artifacts;
mod body;
mod cycles;
mod diagnostic;
mod global;
mod load;
mod record;
mod revision;
mod runner;

pub(crate) use body::{body_has_forbidden_html, body_start_line};
#[cfg(test)]
use diagnostic::local_diagnostic;
use diagnostic::{display_path, global_diagnostic, sort_diagnostics};
use global::validate_global;
pub(crate) use load::collect_files;
use load::load_records;
pub(crate) use record::{
    is_segment, is_semantic_id, normalize_record_bytes, parse_yaml, read_normalized, valid_hash,
    validate_metadata,
};
#[cfg(test)]
use record::{normalize_line_endings, split_frontmatter};
pub(crate) use revision::knowledge_revision;
use revision::snapshot_revision;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckClass {
    Records,
    Relations,
    Authority,
    Artifacts,
}

impl CheckClass {
    pub const ALL: [Self; 4] = [
        Self::Records,
        Self::Relations,
        Self::Authority,
        Self::Artifacts,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Records => "records",
            Self::Relations => "relations",
            Self::Authority => "authority",
            Self::Artifacts => "artifacts",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }

    fn prerequisites(self) -> &'static [Self] {
        match self {
            Self::Records => &[Self::Records],
            Self::Relations => &[Self::Records, Self::Relations],
            Self::Authority => &[Self::Records, Self::Relations, Self::Authority],
            Self::Artifacts => &Self::ALL,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CheckOutcome {
    pub check: CheckClass,
    pub status: CheckStatus,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticPhase {
    Local,
    Global,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    pub phase: DiagnosticPhase,
    pub path: String,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u64>,
    pub affected_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct UnitRevision {
    pub id: String,
    pub revision: String,
    pub path: String,
    pub effective_approval: &'static str,
    pub approval_evidence: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_reason: Option<&'static str>,
    pub eligibility: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub eligibility_evidence: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CheckReport {
    pub schema: &'static str,
    pub ok: bool,
    pub requested_checks: Vec<CheckClass>,
    pub executed_checks: Vec<CheckClass>,
    pub checks: Vec<CheckOutcome>,
    pub authority: &'static str,
    pub approval: &'static str,
    pub checkpoint: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_commit: Option<String>,
    pub snapshot_revision: Option<String>,
    pub affected_ids: Vec<String>,
    pub units: Vec<UnitRevision>,
    pub diagnostics: Vec<Diagnostic>,
    pub next_actions: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !value
}

pub(crate) struct Foundation {
    pub(crate) units: Vec<KnowledgeUnit>,
    pub(crate) owners: Vec<Owner>,
    pub(crate) sources: Vec<Source>,
    pub(crate) negative_records: crate::source::NegativeRecords,
}

pub(crate) fn check_repository(repository_root: &Path) -> CheckReport {
    runner::check_repository_selected(repository_root, &CheckClass::ALL)
}

pub(crate) fn check_repository_selected(
    repository_root: &Path,
    requested: &[CheckClass],
) -> CheckReport {
    runner::check_repository_selected(repository_root, requested)
}

pub(crate) fn load_foundation(repository_root: &Path) -> Result<Foundation, Vec<Diagnostic>> {
    let foundation = load_records(repository_root)?;
    let mut global_diagnostics = validate_global(
        &foundation.units,
        &foundation.owners,
        &foundation.sources,
        &foundation.negative_records,
        repository_root,
    );
    sort_diagnostics(&mut global_diagnostics);
    if !global_diagnostics.is_empty() {
        return Err(global_diagnostics);
    }
    Ok(foundation)
}

#[cfg(test)]
pub(crate) fn failed_authority_report(failure: crate::checkpoint::AuthorityFailure) -> CheckReport {
    runner::failed_authority_report(failure)
}

#[cfg(test)]
mod tests;
