//! Review workflow facade and shared wire-contract types.
//!
//! Callers enter through [`ReviewService`]. Child modules own orchestration,
//! record encoding, filesystem publication, and repository-wide validation.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::check::{Diagnostic, DiagnosticPhase};

const PROJECTION_SCHEMA: &str = "methexis.review-projection/v1alpha1";
const APPROVAL_SCHEMA: &str = "methexis.approval/v1alpha1";
const PROJECTION_REQUEST_SCHEMA: &str = "methexis.review-projection-request/v1alpha1";
const REVIEW_REQUEST_SCHEMA: &str = "methexis.review-request/v1alpha1";
const APPROVAL_REQUEST_SCHEMA: &str = "methexis.approval-request/v1alpha1";
const OPERATION_SCHEMA: &str = "methexis.operation/v1alpha1";
const REVIEW_MANIFEST_SCHEMA: &str = "methexis.review-manifest/v1alpha1";
const PROFILE: &str = "ko-review/v1alpha1";
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_RECORD_BYTES: usize = 256 * 1024;
const COMPILER: &str = concat!("methexis/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug)]
pub(crate) struct ProposalState {
    pub(crate) evidence: &'static str,
    pub(crate) reason: Option<&'static str>,
}

pub(crate) struct ReviewValidation {
    pub(crate) states: BTreeMap<String, ProposalState>,
    pub(crate) evidence: BTreeMap<String, ApprovalEvidence>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug)]
pub(crate) struct ApprovalEvidence {
    pub(crate) projection_hash: String,
    pub(crate) projection_profile: String,
    pub(crate) projection_compiler: String,
    pub(crate) approval_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectionRequest {
    schema: String,
    knowledge_id: String,
    expected_revision: String,
    korean_markdown: String,
    #[serde(default)]
    replace_projection_hash: Option<String>,
}

#[derive(Serialize)]
struct ProjectionInput<'a> {
    schema: &'static str,
    knowledge_id: &'a str,
    expected_revision: &'a str,
    korean_markdown: &'a str,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewRequest {
    schema: String,
    knowledge_id: String,
    expected_revision: String,
    projection_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRequest {
    schema: String,
    knowledge_id: String,
    expected_revision: String,
    projection_hash: String,
    reviewer: String,
    reviewed_at: String,
    #[serde(default)]
    replace_revision: Option<String>,
}

#[derive(Serialize)]
struct ApprovalInput<'a> {
    schema: &'static str,
    knowledge_id: &'a str,
    expected_revision: &'a str,
    projection_hash: &'a str,
    reviewer: &'a str,
    reviewed_at: &'a str,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionMetadata {
    schema: String,
    knowledge_id: String,
    revision: String,
    profile: String,
    compiler: String,
    request_hash: String,
}

#[derive(Clone, Debug)]
struct ProjectionRecord {
    metadata: ProjectionMetadata,
    path: PathBuf,
    hash: String,
    body: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRecord {
    schema: String,
    knowledge_id: String,
    revision: String,
    reviewer: String,
    reviewed_at: String,
    projection_profile: String,
    projection_compiler: String,
    projection_hash: String,
    request_hash: String,
    #[serde(skip)]
    hash: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct OperationSuccess {
    schema: &'static str,
    pub(crate) ok: bool,
    operation: &'static str,
    status: &'static str,
    authority: &'static str,
    affected_ids: Vec<String>,
    path: String,
    hash: String,
    request_hash: String,
    next_actions: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct OperationFailure {
    schema: &'static str,
    pub(crate) ok: bool,
    operation: &'static str,
    error: Box<OperationErrorBody>,
}

#[derive(Clone, Debug, Serialize)]
struct OperationErrorBody {
    code: String,
    message: String,
    affected_ids: Vec<String>,
    next_actions: Vec<String>,
}

impl OperationFailure {
    fn new(
        operation: &'static str,
        code: impl Into<String>,
        message: impl Into<String>,
        affected_ids: Vec<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            schema: OPERATION_SCHEMA,
            ok: false,
            operation,
            error: Box::new(OperationErrorBody {
                code: code.into(),
                message: message.into(),
                affected_ids,
                next_actions: vec![next_action.into()],
            }),
        }
    }
}

#[derive(Serialize)]
struct ReviewManifest<'a> {
    schema: &'static str,
    review_id: &'a str,
    knowledge_id: &'a str,
    revision: &'a str,
    projection_hash: &'a str,
    request_hash: &'a str,
    source_status: &'static str,
    packet_path: &'a str,
    packet_hash: &'a str,
}

struct SuccessInput<'a> {
    operation: &'static str,
    status: &'static str,
    repository_root: &'a Path,
    path: &'a Path,
    hash: String,
    request_hash: String,
    next_actions: Vec<String>,
    id: String,
}

mod operations;
mod records;
mod storage;
mod validation;

pub(crate) struct ReviewService<'a> {
    repository_root: &'a Path,
}

impl<'a> ReviewService<'a> {
    pub(crate) fn new(repository_root: &'a Path) -> Self {
        Self { repository_root }
    }

    pub(crate) fn generate_projection(
        &self,
        request_path: &Path,
    ) -> Result<OperationSuccess, OperationFailure> {
        operations::generate_projection(self.repository_root, request_path)
    }

    pub(crate) fn build_review(
        &self,
        request_path: &Path,
    ) -> Result<OperationSuccess, OperationFailure> {
        operations::build_review(self.repository_root, request_path)
    }

    pub(crate) fn record_approval(
        &self,
        request_path: &Path,
    ) -> Result<OperationSuccess, OperationFailure> {
        operations::record_approval(self.repository_root, request_path)
    }
}

pub(crate) use validation::validate_records;

fn success(input: SuccessInput<'_>) -> OperationSuccess {
    OperationSuccess {
        schema: OPERATION_SCHEMA,
        ok: true,
        operation: input.operation,
        status: input.status,
        authority: "draft_proposal",
        affected_ids: vec![input.id],
        path: relative_path(input.repository_root, input.path),
        hash: input.hash,
        request_hash: input.request_hash,
        next_actions: input.next_actions,
    }
}

fn failure_from_diagnostic(
    operation: &'static str,
    diagnostic: Diagnostic,
    next_action: &str,
) -> OperationFailure {
    OperationFailure::new(
        operation,
        diagnostic.code,
        diagnostic.message,
        diagnostic.affected_ids,
        next_action,
    )
}

fn local_diagnostic(
    path: String,
    code: impl Into<String>,
    message: String,
    affected_ids: Vec<String>,
) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Local,
        path,
        code: code.into(),
        message,
        line: None,
        column: None,
        affected_ids,
    }
}

fn global_diagnostic(
    repository_root: &Path,
    path: &Path,
    code: impl Into<String>,
    message: String,
    affected_ids: Vec<String>,
) -> Diagnostic {
    Diagnostic {
        phase: DiagnosticPhase::Global,
        path: relative_path(repository_root, path),
        code: code.into(),
        message,
        line: None,
        column: None,
        affected_ids,
    }
}

fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|left, right| {
        (
            left.phase,
            &left.path,
            &left.code,
            left.line,
            left.column,
            &left.message,
            &left.affected_ids,
        )
            .cmp(&(
                right.phase,
                &right.path,
                &right.code,
                right.line,
                right.column,
                &right.message,
                &right.affected_ids,
            ))
    });
}

fn semantic_hash(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("request structs serialize");
    hash_bytes(&bytes)
}

fn hash_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_review_time(value: &str) -> bool {
    let syntactically_valid = value.len() == 20
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
        && value.as_bytes().get(16) == Some(&b':')
        && value.ends_with('Z')
        && value.bytes().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        });
    if !syntactically_valid {
        return false;
    }

    let number =
        |range: std::ops::Range<usize>| value.get(range).and_then(|part| part.parse::<u32>().ok());
    let Some(year) = number(0..4) else {
        return false;
    };
    let Some(month @ 1..=12) = number(5..7) else {
        return false;
    };
    let Some(day) = number(8..10) else {
        return false;
    };
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };

    year > 0
        && (1..=days_in_month).contains(&day)
        && matches!(number(11..13), Some(0..=23))
        && matches!(number(14..16), Some(0..=59))
        && matches!(number(17..19), Some(0..=59))
}

fn relative_path(repository_root: &Path, path: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
