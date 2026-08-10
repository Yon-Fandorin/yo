//! Semantic-first authoring contract.
//!
//! This version publishes only canonical Source and Knowledge records. Korean
//! Projection and packet publication remain explicit later operations.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AuthorSuccess, decode_request, shared};
use crate::review::{OperationFailure, semantic_hash};

pub(super) const REQUEST_SCHEMA: &str = "methexis.author-revision-request/v1alpha2";
const SUCCESS_SCHEMA: &str = "methexis.author-revision/v1alpha2";
const OPERATION: &str = "author-revision";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    knowledge_id: String,
    expected_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    knowledge_body: Option<String>,
    // Retaining this named field lets callers receive a contract-specific
    // migration error instead of a generic unknown-field parse failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    korean_markdown: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Success {
    schema: &'static str,
    ok: bool,
    operation: &'static str,
    status: &'static str,
    authority: &'static str,
    affected_ids: Vec<String>,
    revision: String,
    changed_paths: Vec<String>,
    request_hash: String,
    next_actions: Vec<String>,
}

pub(super) fn author_revision(
    repository_root: &Path,
    bytes: &[u8],
) -> Result<AuthorSuccess, OperationFailure> {
    let request: Request = decode_request(bytes)?;
    if request.korean_markdown.is_some() {
        return Err(OperationFailure::new(
            OPERATION,
            "korean_markdown_forbidden",
            "v1alpha2 authors Source and Knowledge only; it does not accept korean_markdown",
            vec![request.knowledge_id],
            "run project-review explicitly after repository review of the semantic candidate",
        ));
    }
    if request.source_content.is_none() && request.knowledge_body.is_none() {
        return Err(OperationFailure::new(
            OPERATION,
            "empty_author_request",
            "at least one of source_content or knowledge_body is required",
            vec![request.knowledge_id],
            "provide the new Source content or Knowledge body",
        ));
    }
    let request_hash = semantic_hash(&request);
    let normalized = shared::normalize(
        repository_root,
        shared::SemanticInput {
            knowledge_id: request.knowledge_id.clone(),
            expected_revision: request.expected_revision,
            source_content: request.source_content,
            knowledge_body: request.knowledge_body,
        },
    )?;
    let prepared = shared::derive_and_capture(repository_root, normalized)?;
    let mut changed_paths = Vec::new();
    let written = shared::publish_semantic(repository_root, &prepared, &mut changed_paths)?;

    Ok(AuthorSuccess::V1Alpha2(Success {
        schema: SUCCESS_SCHEMA,
        ok: true,
        operation: OPERATION,
        status: if written { "written" } else { "unchanged" },
        authority: "draft_proposal",
        affected_ids: vec![prepared.id],
        revision: prepared.revision,
        changed_paths,
        request_hash,
        next_actions: vec![
            "run methexis check --only records,relations, commit the semantic candidate, and complete repository review before project-review"
                .to_owned(),
        ],
    }))
}
