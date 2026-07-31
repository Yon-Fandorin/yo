//! Trusted Git snapshot and Checkpoint workflow facade.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CREATE_REQUEST_SCHEMA: &str = "methexis.checkpoint-request/v1alpha1";
const ACTIVATE_REQUEST_SCHEMA: &str = "methexis.activation-request/v1alpha1";
const CHECKPOINT_SCHEMA: &str = "methexis.checkpoint/v1alpha1";
const ACTIVE_SCHEMA: &str = "methexis.active-checkpoint/v1alpha1";
const OPERATION_SCHEMA: &str = "methexis.operation/v1alpha1";
const DEFAULT_TRUSTED_REF: &str = "refs/heads/develop";
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_RECORD_BYTES: usize = 256 * 1024;

mod context;
mod evaluation;
mod git;
mod operations;
mod prospective;
mod records;
mod storage;
mod validation;

pub(crate) use context::{
    ContextAuthority, final_revalidate as final_revalidate_context_authority,
    resolve as resolve_context_authority,
};
pub(crate) use evaluation::{ActiveCheckpoint, AuthorityFailure};

pub(crate) enum StagedTransition {
    Prospective(prospective::ProspectiveActivation),
    Ordinary(StagedFallback),
}

pub(crate) struct StagedFallback {
    index: git::ProposalIndex,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    schema: String,
    roots: Vec<String>,
}

#[derive(Serialize)]
struct CreateInput<'a> {
    schema: &'static str,
    roots: &'a [String],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ActivationRequest {
    schema: String,
    checkpoint_id: String,
    checkpoint_hash: String,
    #[serde(default)]
    replace_active_hash: Option<String>,
}

#[derive(Serialize)]
struct ActivationInput<'a> {
    schema: &'static str,
    checkpoint_id: &'a str,
    checkpoint_hash: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    replace_active_hash: Option<&'a str>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CheckpointRecord {
    schema: String,
    checkpoint_id: String,
    trusted_commit: String,
    source_status: String,
    roots: Vec<String>,
    units: Vec<CheckpointUnit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CheckpointUnit {
    id: String,
    revision: String,
    reasons: Vec<String>,
}

#[derive(Serialize)]
struct CheckpointIdentity<'a> {
    schema: &'static str,
    trusted_commit: &'a str,
    source_status: &'static str,
    roots: &'a [String],
    units: &'a [CheckpointUnit],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ActiveRecord {
    schema: String,
    checkpoint_id: String,
    checkpoint_hash: String,
    trusted_commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replaces_active_hash: Option<String>,
    request_hash: String,
}

#[derive(Serialize)]
struct ActiveIdentity<'a> {
    schema: &'static str,
    checkpoint_id: &'a str,
    checkpoint_hash: &'a str,
    trusted_commit: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    replaces_active_hash: Option<&'a str>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct OperationSuccess {
    schema: &'static str,
    pub(crate) ok: bool,
    operation: &'static str,
    status: &'static str,
    authority: &'static str,
    trusted_commit: String,
    affected_ids: Vec<String>,
    path: String,
    hash: String,
    checkpoint_id: String,
    request_hash: String,
    next_actions: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct OperationFailure {
    schema: &'static str,
    pub(crate) ok: bool,
    operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    trusted_commit: Option<String>,
    error: Box<OperationError>,
}

#[derive(Clone, Debug, Serialize)]
struct OperationError {
    code: String,
    message: String,
    affected_ids: Vec<String>,
    next_actions: Vec<String>,
}

impl OperationFailure {
    fn new(
        operation: &'static str,
        trusted_commit: Option<String>,
        code: impl Into<String>,
        message: impl Into<String>,
        affected_ids: Vec<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            schema: OPERATION_SCHEMA,
            ok: false,
            operation,
            trusted_commit,
            error: Box::new(OperationError {
                code: code.into(),
                message: message.into(),
                affected_ids,
                next_actions: vec![next_action.into()],
            }),
        }
    }

    fn code(&self) -> &str {
        &self.error.code
    }
}

pub(crate) use evaluation::evaluate;

pub(crate) struct CheckpointService<'a> {
    repository_root: &'a Path,
    trusted_ref: &'a str,
}

impl<'a> CheckpointService<'a> {
    pub(crate) fn new(repository_root: &'a Path) -> Self {
        Self {
            repository_root,
            trusted_ref: DEFAULT_TRUSTED_REF,
        }
    }

    pub(crate) fn create(&self, request_path: &Path) -> Result<OperationSuccess, OperationFailure> {
        operations::create(self.repository_root, self.trusted_ref, request_path)
    }

    pub(crate) fn propose_activation(
        &self,
        request_path: &Path,
    ) -> Result<OperationSuccess, OperationFailure> {
        operations::propose_activation(self.repository_root, self.trusted_ref, request_path)
    }

    pub(crate) fn check_staged_transition(&self) -> Result<StagedTransition, OperationFailure> {
        prospective::check_staged(self.repository_root, self.trusted_ref)
    }

    pub(crate) fn finish_staged_fallback(
        &self,
        fallback: StagedFallback,
    ) -> Result<(), OperationFailure> {
        git::ensure_index_unchanged(
            self.repository_root,
            &fallback.index,
            prospective::OPERATION,
        )
    }
}

fn semantic_hash(value: &impl Serialize) -> String {
    hash_bytes(&serde_json::to_vec(value).expect("closed request structs serialize"))
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

fn relative_path(repository_root: &Path, path: &Path) -> String {
    path.strip_prefix(repository_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod context_tests;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

struct SelectedCheckpoint {
    roots: Vec<String>,
    units: Vec<CheckpointUnit>,
}
