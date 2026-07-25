//! Versioned agent request, result, and failure contracts.

use serde::{Deserialize, Serialize};

pub(crate) const REQUEST_SCHEMA: &str = "librarian.discovery-request/v1alpha1";
pub(crate) const RESULT_SCHEMA: &str = "librarian.candidate-set/v1alpha1";
pub(crate) const ERROR_SCHEMA: &str = "librarian.error/v1alpha1";
pub(crate) const COMPILER: &str = concat!("librarian/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscoveryRequest {
    pub(crate) schema: String,
    #[serde(default)]
    pub(crate) query: Option<String>,
    #[serde(default)]
    pub(crate) anchors: Vec<Anchor>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Anchor {
    KnowledgeId { value: String },
    Path { value: String },
    Symbol { value: String },
}

impl Anchor {
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::KnowledgeId { .. } => "knowledge_id",
            Self::Path { .. } => "path",
            Self::Symbol { .. } => "symbol",
        }
    }

    pub(crate) fn value(&self) -> &str {
        match self {
            Self::KnowledgeId { value } | Self::Path { value } | Self::Symbol { value } => value,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct CandidateSet {
    pub(crate) schema: &'static str,
    pub(crate) ok: bool,
    pub(crate) candidate_set_id: String,
    pub(crate) request_hash: String,
    pub(crate) catalog_hash: String,
    pub(crate) compiler: &'static str,
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) unresolved_anchors: Vec<Anchor>,
    pub(crate) truncated: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct Candidate {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) score: u64,
    pub(crate) reasons: Vec<CandidateReason>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CandidateReason {
    Anchor {
        anchor_kind: &'static str,
        value: String,
        field: &'static str,
        score: u64,
    },
    ExactQuery {
        field: &'static str,
        score: u64,
    },
    QueryPhrase {
        field: &'static str,
        score: u64,
    },
    QueryToken {
        field: &'static str,
        term: String,
        score: u64,
    },
    Relation {
        via: String,
        score: u64,
    },
}

#[derive(Debug, Serialize)]
pub(crate) struct FailureEnvelope {
    pub(crate) schema: &'static str,
    pub(crate) ok: bool,
    pub(crate) operation: &'static str,
    pub(crate) error: FailureBody,
}

#[derive(Debug, Serialize)]
pub(crate) struct FailureBody {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "is_false")]
    pub(crate) retryable: bool,
    pub(crate) affected_ids: Vec<String>,
    pub(crate) affected_paths: Vec<String>,
    pub(crate) next_actions: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !value
}
