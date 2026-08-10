//! Original all-in-one authoring contract.
//!
//! Its request and response bytes remain compatible while implementation
//! delegates canonical Source and Knowledge work to the shared layer.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{AuthorSuccess, decode_request, shared};
use crate::review::{
    COMPILER, OperationFailure, PROFILE, PROJECTION_SCHEMA, ProjectionMetadata, ProjectionRecord,
    hash_bytes,
    operations::{normalize_markdown, publish_review_packet, require_schema},
    records::{parse_projection, projection_input_hash, render_projection, render_projection_body},
    relative_path, semantic_hash,
    storage::publish_tracked_record,
};

pub(super) const REQUEST_SCHEMA: &str = "methexis.author-revision-request/v1alpha1";
const SUCCESS_SCHEMA: &str = "methexis.author-revision/v1alpha1";
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    korean_markdown: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct PacketRef {
    path: String,
    hash: String,
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
    projection_hash: String,
    packet: PacketRef,
    changed_paths: Vec<String>,
    request_hash: String,
    next_actions: Vec<String>,
}

pub(super) fn author_revision(
    repository_root: &Path,
    bytes: &[u8],
) -> Result<AuthorSuccess, OperationFailure> {
    let request: Request = decode_request(bytes)?;
    require_schema(
        OPERATION,
        &request.schema,
        REQUEST_SCHEMA,
        &request.knowledge_id,
    )?;
    if request.source_content.is_none()
        && request.knowledge_body.is_none()
        && request.korean_markdown.is_none()
    {
        return Err(OperationFailure::new(
            OPERATION,
            "empty_author_request",
            "at least one of source_content, knowledge_body, or korean_markdown is required",
            vec![request.knowledge_id],
            "provide the new Source content, Knowledge body, or Korean Markdown",
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
    let korean = request
        .korean_markdown
        .as_deref()
        .map(|markdown| normalize_markdown(markdown, OPERATION, &request.knowledge_id))
        .transpose()?;
    let prepared = shared::derive_and_capture(repository_root, normalized)?;

    let projection_path = repository_root
        .join("methexis/review-projections")
        .join(format!("{}.md", request.knowledge_id));
    let korean = match korean {
        Some(korean) => korean,
        None => parse_projection(&projection_path, repository_root)
            .map_err(|diagnostic| {
                crate::review::failure_from_diagnostic(
                    OPERATION,
                    diagnostic,
                    "provide korean_markdown to author the replacement Projection",
                )
            })?
            .translation()
            .to_owned(),
    };
    let projection_request_hash =
        projection_input_hash(&request.knowledge_id, &prepared.revision, &korean);
    let projection_bytes =
        render_projection(&prepared.authored_unit, &projection_request_hash, &korean);
    let projection_hash = hash_bytes(&projection_bytes);
    let projection = ProjectionRecord {
        metadata: ProjectionMetadata {
            schema: PROJECTION_SCHEMA.to_owned(),
            knowledge_id: request.knowledge_id.clone(),
            revision: prepared.revision.clone(),
            profile: PROFILE.to_owned(),
            compiler: COMPILER.to_owned(),
            request_hash: projection_request_hash,
        },
        path: projection_path.clone(),
        hash: projection_hash.clone(),
        body: render_projection_body(&korean),
    };
    let projection_capture = match std::fs::symlink_metadata(&projection_path) {
        Ok(_) => Some(shared::capture(
            repository_root,
            &projection_path,
            &request.knowledge_id,
        )?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(OperationFailure::new(
                OPERATION,
                "publication_failed",
                format!("cannot inspect existing Projection: {error}"),
                vec![request.knowledge_id],
                "repair the destination and retry",
            ));
        },
    };

    let mut changed_paths = Vec::new();
    let mut written = shared::publish_semantic(repository_root, &prepared, &mut changed_paths)?;
    written |= shared::publish_result(
        repository_root,
        publish_tracked_record(
            repository_root,
            &projection_path,
            &projection_bytes,
            projection_capture
                .as_ref()
                .map(|capture| hash_bytes(capture.bytes()))
                .as_deref(),
            OPERATION,
            &request.knowledge_id,
            "Projection",
            "re-run the same request to rebuild from current state",
            "re-run the same request to rebuild from current state",
        ),
        &projection_path,
        &mut changed_paths,
    )?;
    let published = publish_review_packet(
        repository_root,
        OPERATION,
        &prepared.authored_unit,
        &projection,
    )
    .map_err(|failure| failure.into_partial(&changed_paths))?;
    written |= published.status == "written";

    Ok(AuthorSuccess::V1Alpha1(Success {
        schema: SUCCESS_SCHEMA,
        ok: true,
        operation: OPERATION,
        status: if written { "written" } else { "unchanged" },
        authority: "draft_proposal",
        affected_ids: vec![request.knowledge_id],
        revision: prepared.revision,
        projection_hash,
        packet: PacketRef {
            path: relative_path(repository_root, &published.manifest_path),
            hash: published.packet_hash,
        },
        changed_paths,
        request_hash,
        next_actions: vec![
            "review the tracked diff and packet, then record approval with methexis approve"
                .to_owned(),
        ],
    }))
}
