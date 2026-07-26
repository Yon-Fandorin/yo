//! Versioned Context Resolution request, result, and Librarian wire contracts.

use serde::{Deserialize, Serialize};

pub(super) const REQUEST_SCHEMA: &str = "methexis.context-request/v1alpha1";
pub(super) const RESULT_SCHEMA: &str = "methexis.context-result/v1alpha1";
pub(super) const FAILURE_SCHEMA: &str = "methexis.context-failure/v1alpha1";
pub(super) const CANDIDATE_SCHEMA: &str = "librarian.candidate-set/v1alpha1";
pub(super) const TOKENIZER_PROFILE: &str = "o200k_base/v1";
pub(super) const PAYLOAD_PROFILE: &str = "methexis.context-markdown/v1alpha1";
pub(super) const COMPILER: &str = concat!("methexis/", env!("CARGO_PKG_VERSION"));
pub(super) const TOKENIZER_COMPILER: &str = "tiktoken-rs/0.12.0";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveRequest {
    pub(super) schema: String,
    #[serde(default)]
    pub(super) anchors: Vec<Anchor>,
    #[serde(default)]
    pub(super) candidates: Option<CandidateReference>,
    pub(super) tokenizer_profile: String,
    pub(super) max_tokens: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Anchor {
    KnowledgeId { value: String },
    Path { value: String },
    Symbol { value: String },
}

impl Anchor {
    pub(super) const fn kind(&self) -> &'static str {
        match self {
            Self::KnowledgeId { .. } => "knowledge_id",
            Self::Path { .. } => "path",
            Self::Symbol { .. } => "symbol",
        }
    }

    pub(super) fn value(&self) -> &str {
        match self {
            Self::KnowledgeId { value } | Self::Path { value } | Self::Symbol { value } => value,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CandidateReference {
    pub(super) path: String,
    pub(super) hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CandidateSet {
    pub(super) schema: String,
    pub(super) ok: bool,
    pub(super) candidate_set_id: String,
    pub(super) request_hash: String,
    pub(super) catalog_hash: String,
    pub(super) compiler: String,
    pub(super) candidates: Vec<Candidate>,
    pub(super) unresolved_anchors: Vec<Anchor>,
    pub(super) truncated: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Candidate {
    pub(super) id: String,
    pub(super) path: String,
    pub(super) score: u64,
    pub(super) reasons: Vec<CandidateReason>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CandidateReason {
    Anchor {
        anchor_kind: String,
        value: String,
        field: String,
        score: u64,
    },
    ExactQuery {
        field: String,
        score: u64,
    },
    QueryPhrase {
        field: String,
        score: u64,
    },
    QueryToken {
        field: String,
        term: String,
        score: u64,
    },
    Relation {
        via: String,
        score: u64,
    },
}

impl CandidateReason {
    pub(super) const fn score(&self) -> u64 {
        match self {
            Self::Anchor { score, .. }
            | Self::ExactQuery { score, .. }
            | Self::QueryPhrase { score, .. }
            | Self::QueryToken { score, .. }
            | Self::Relation { score, .. } => *score,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ArtifactReference {
    pub(super) path: String,
    pub(super) hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tokens: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ResolveSuccess {
    schema: &'static str,
    pub(crate) ok: bool,
    operation: &'static str,
    status: &'static str,
    authority: &'static str,
    trusted_commit: String,
    build_id: String,
    context: ArtifactReference,
    manifest: ArtifactReference,
    affected_ids: Vec<String>,
    next_actions: Vec<String>,
}

impl ResolveSuccess {
    pub(super) fn new(
        status: &'static str,
        trusted_commit: String,
        build_id: String,
        context: ArtifactReference,
        manifest: ArtifactReference,
        affected_ids: Vec<String>,
    ) -> Self {
        Self {
            schema: RESULT_SCHEMA,
            ok: true,
            operation: "resolve_context",
            status,
            authority: "trusted_integration",
            trusted_commit,
            build_id,
            context,
            manifest,
            affected_ids,
            next_actions: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ResolveFailure {
    schema: &'static str,
    pub(crate) ok: bool,
    operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    trusted_commit: Option<String>,
    error: Box<ResolveError>,
}

#[derive(Clone, Debug, Serialize)]
struct ResolveError {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "is_false")]
    retryable: bool,
    affected_ids: Vec<String>,
    affected_paths: Vec<String>,
    next_actions: Vec<String>,
}

impl ResolveFailure {
    pub(super) fn new(
        trusted_commit: Option<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        affected_ids: Vec<String>,
        affected_paths: Vec<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            schema: FAILURE_SCHEMA,
            ok: false,
            operation: "resolve_context",
            trusted_commit,
            error: Box::new(ResolveError {
                code: code.into(),
                message: message.into(),
                retryable,
                affected_ids,
                affected_paths,
                next_actions: vec![next_action.into()],
            }),
        }
    }

    pub(super) fn with_trusted_commit(mut self, trusted_commit: &str) -> Self {
        if self.trusted_commit.is_none() {
            self.trusted_commit = Some(trusted_commit.to_owned());
        }
        self
    }

    #[cfg(test)]
    pub(super) fn code(&self) -> &str {
        &self.error.code
    }
}

const fn is_false(value: &bool) -> bool {
    !*value
}
