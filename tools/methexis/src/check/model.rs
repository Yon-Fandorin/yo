use serde::Serialize;

use crate::{
    model::{KnowledgeUnit, Owner, Source},
    source,
};

fn is_false(value: &bool) -> bool {
    !value
}

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

    pub(super) fn prerequisites(self) -> &'static [Self] {
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

pub(crate) struct Foundation {
    pub(crate) units: Vec<KnowledgeUnit>,
    pub(crate) owners: Vec<Owner>,
    pub(crate) sources: Vec<Source>,
    pub(crate) negative_records: source::NegativeRecords,
}
