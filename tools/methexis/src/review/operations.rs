//! Agent operations that coordinate foundation, record, and storage boundaries.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use super::{
    APPROVAL_REQUEST_SCHEMA, ApprovalInput, ApprovalRequest, CANONICAL_APPROVAL_REQUEST_SCHEMA,
    CanonicalApprovalInput, MAX_REQUEST_BYTES, OperationFailure, OperationSuccess,
    PROJECTION_REQUEST_SCHEMA, ProjectionRecord, ProjectionRequest, REVIEW_MANIFEST_SCHEMA,
    REVIEW_REQUEST_SCHEMA, ReviewManifest, ReviewRequest, SuccessInput, failure_from_diagnostic,
    hash_bytes,
    records::{
        parse_projection, projection_input_hash, render_approval, render_projection,
        render_review_packet,
    },
    relative_path, semantic_hash,
    storage::{publish_approval, publish_artifact_directory, publish_tracked},
    success, valid_review_time,
};
use crate::{
    check::{Foundation, load_foundation},
    model::KnowledgeUnit,
};

pub(super) fn generate_projection(
    repository_root: &Path,
    request_path: &Path,
) -> Result<OperationSuccess, OperationFailure> {
    const OPERATION: &str = "generate_review_projection";
    let request: ProjectionRequest = read_request(request_path, OPERATION)?;
    require_schema(
        OPERATION,
        &request.schema,
        PROJECTION_REQUEST_SCHEMA,
        &request.knowledge_id,
    )?;
    let foundation = load_operation_foundation(repository_root, OPERATION, &request.knowledge_id)?;
    let unit = require_unit(
        &foundation,
        &request.knowledge_id,
        &request.expected_revision,
        OPERATION,
    )?;
    let korean = normalize_markdown(&request.korean_markdown, OPERATION, &request.knowledge_id)?;
    let request_hash =
        projection_input_hash(&request.knowledge_id, &request.expected_revision, &korean);
    let bytes = render_projection(unit, &request_hash, &korean);
    let hash = hash_bytes(&bytes);
    let target = repository_root
        .join("methexis/review-projections")
        .join(format!("{}.md", request.knowledge_id));
    let status = publish_tracked(
        repository_root,
        &target,
        &bytes,
        request.replace_projection_hash.as_deref(),
        OPERATION,
        &request.knowledge_id,
    )?;

    Ok(success(SuccessInput {
        operation: OPERATION,
        status,
        repository_root,
        path: &target,
        hash,
        request_hash,
        next_actions: vec!["build a review packet before requesting approval".to_owned()],
        id: request.knowledge_id,
    }))
}

pub(super) fn build_review(
    repository_root: &Path,
    request_path: &Path,
) -> Result<OperationSuccess, OperationFailure> {
    const OPERATION: &str = "build_review";
    let request: ReviewRequest = read_request(request_path, OPERATION)?;
    require_schema(
        OPERATION,
        &request.schema,
        REVIEW_REQUEST_SCHEMA,
        &request.knowledge_id,
    )?;
    let foundation = load_operation_foundation(repository_root, OPERATION, &request.knowledge_id)?;
    let unit = require_unit(
        &foundation,
        &request.knowledge_id,
        &request.expected_revision,
        OPERATION,
    )?;
    let projection_path = repository_root
        .join("methexis/review-projections")
        .join(format!("{}.md", request.knowledge_id));
    let projection = parse_projection(&projection_path, repository_root).map_err(|diagnostic| {
        failure_from_diagnostic(
            OPERATION,
            diagnostic,
            "generate the matching Projection first",
        )
    })?;
    require_projection_match(OPERATION, unit, &projection, &request.projection_hash)?;

    let published = publish_review_packet(repository_root, OPERATION, unit, &projection)?;

    Ok(success(SuccessInput {
        operation: OPERATION,
        status: published.status,
        repository_root,
        path: &published.manifest_path,
        hash: published.packet_hash,
        request_hash: published.request_hash,
        next_actions: vec!["read the packet and obtain explicit human approval".to_owned()],
        id: unit.metadata.id.clone(),
    }))
}

pub(crate) struct PublishedPacket {
    pub(crate) status: &'static str,
    pub(crate) manifest_path: PathBuf,
    pub(crate) packet_hash: String,
    pub(crate) request_hash: String,
}

/// Builds and publishes the deterministic review packet for one exact end
/// state. Shared by `build-review` and `author-revision` so both produce
/// byte-identical packets for the same unit and Projection.
pub(crate) fn publish_review_packet(
    repository_root: &Path,
    operation: &'static str,
    unit: &KnowledgeUnit,
    projection: &ProjectionRecord,
) -> Result<PublishedPacket, OperationFailure> {
    let request = ReviewRequest {
        schema: REVIEW_REQUEST_SCHEMA.to_owned(),
        knowledge_id: unit.metadata.id.clone(),
        expected_revision: unit.revision.clone(),
        projection_hash: projection.hash.clone(),
    };
    let request_hash = semantic_hash(&request);
    let packet = render_review_packet(unit, projection);
    let packet_hash = hash_bytes(packet.as_bytes());
    let review_id = hash_bytes(
        format!(
            "{}\n{}\n{}\n{}",
            unit.metadata.id, unit.revision, projection.hash, request_hash
        )
        .as_bytes(),
    );
    let directory = repository_root
        .join(".local-exclude/methexis/reviews")
        .join(review_id.trim_start_matches("sha256:"));
    let packet_relative = relative_path(repository_root, &directory.join("packet.md"));
    let manifest = ReviewManifest {
        schema: REVIEW_MANIFEST_SCHEMA.to_owned(),
        review_id: review_id.clone(),
        knowledge_id: unit.metadata.id.clone(),
        revision: unit.revision.clone(),
        projection_hash: projection.hash.clone(),
        request_hash: request_hash.clone(),
        source_status: "not_evaluated".to_owned(),
        packet_path: packet_relative,
        packet_hash: packet_hash.clone(),
    };
    let mut manifest_bytes = serde_json::to_vec(&manifest).map_err(|error| {
        OperationFailure::new(
            operation,
            "serialization_failed",
            error.to_string(),
            vec![unit.metadata.id.clone()],
            "report the compiler failure",
        )
    })?;
    manifest_bytes.push(b'\n');
    let status = publish_artifact_directory(
        repository_root,
        &directory,
        &[
            ("packet.md", packet.as_bytes()),
            ("manifest.json", &manifest_bytes),
        ],
        operation,
        &unit.metadata.id,
    )?;

    Ok(PublishedPacket {
        status,
        manifest_path: directory.join("manifest.json"),
        packet_hash,
        request_hash,
    })
}

pub(super) fn record_approval(
    repository_root: &Path,
    request_path: &Path,
) -> Result<OperationSuccess, OperationFailure> {
    const OPERATION: &str = "record_approval";
    let request: ApprovalRequest = read_request(request_path, OPERATION)?;
    if !valid_review_time(request.reviewed_at()) {
        return Err(OperationFailure::new(
            OPERATION,
            "invalid_review_time",
            "reviewed_at must use UTC `YYYY-MM-DDTHH:MM:SSZ`",
            vec![request.knowledge_id().to_owned()],
            "provide the explicit human review time in UTC",
        ));
    }
    let foundation = load_operation_foundation(repository_root, OPERATION, request.knowledge_id())?;
    let unit = require_unit(
        &foundation,
        request.knowledge_id(),
        request.expected_revision(),
        OPERATION,
    )?;
    if !foundation
        .owners
        .iter()
        .any(|owner| owner.id == request.reviewer())
    {
        return Err(OperationFailure::new(
            OPERATION,
            "unknown_reviewer",
            format!("reviewer OwnerId `{}` does not exist", request.reviewer()),
            vec![request.knowledge_id().to_owned()],
            "use a tracked OwnerId",
        ));
    }
    let (request_hash, projection) = match &request {
        ApprovalRequest::Projection {
            knowledge_id,
            expected_revision,
            projection_hash,
            reviewer,
            reviewed_at,
            ..
        } => {
            let projection_path = repository_root
                .join("methexis/review-projections")
                .join(format!("{knowledge_id}.md"));
            let projection =
                parse_projection(&projection_path, repository_root).map_err(|diagnostic| {
                    failure_from_diagnostic(
                        OPERATION,
                        diagnostic,
                        "generate the matching Projection first",
                    )
                })?;
            require_projection_match(OPERATION, unit, &projection, projection_hash)?;
            let request_hash = semantic_hash(&ApprovalInput {
                schema: APPROVAL_REQUEST_SCHEMA,
                knowledge_id,
                expected_revision,
                projection_hash,
                reviewer,
                reviewed_at,
            });
            (request_hash, Some(projection))
        },
        ApprovalRequest::Canonical {
            knowledge_id,
            expected_revision,
            review_basis,
            reviewer,
            reviewed_at,
            ..
        } => (
            semantic_hash(&CanonicalApprovalInput {
                schema: CANONICAL_APPROVAL_REQUEST_SCHEMA,
                knowledge_id,
                expected_revision,
                review_basis: *review_basis,
                reviewer,
                reviewed_at,
            }),
            None,
        ),
    };
    let bytes = render_approval(&request, projection.as_ref(), &request_hash);
    let hash = hash_bytes(&bytes);
    let target = repository_root
        .join("methexis/approvals")
        .join(format!("{}.yaml", request.knowledge_id()));
    let status = publish_approval(
        repository_root,
        &target,
        &bytes,
        request.replace_revision(),
        OPERATION,
        request.knowledge_id(),
    )?;

    Ok(success(SuccessInput {
        operation: OPERATION,
        status,
        repository_root,
        path: &target,
        hash,
        request_hash,
        next_actions: vec![
            "submit the approval proposal through repository review; it is not authoritative yet"
                .to_owned(),
        ],
        id: request.knowledge_id().to_owned(),
    }))
}

pub(crate) fn load_operation_foundation(
    repository_root: &Path,
    operation: &'static str,
    id: &str,
) -> Result<Foundation, OperationFailure> {
    load_foundation(repository_root).map_err(|diagnostics| {
        let message = diagnostics.first().map_or_else(
            || "foundation validation failed".to_owned(),
            |item| item.message.clone(),
        );
        OperationFailure::new(
            operation,
            "foundation_invalid",
            message,
            vec![id.to_owned()],
            "run `methexis check` and repair KnowledgeUnit or Owner records",
        )
    })
}

pub(crate) fn require_unit<'a>(
    foundation: &'a Foundation,
    id: &str,
    expected_revision: &str,
    operation: &'static str,
) -> Result<&'a KnowledgeUnit, OperationFailure> {
    let Some(unit) = foundation.units.iter().find(|unit| unit.metadata.id == id) else {
        return Err(OperationFailure::new(
            operation,
            "unknown_knowledge_id",
            format!("KnowledgeId `{id}` does not exist"),
            vec![id.to_owned()],
            "use an ID reported by `methexis check`",
        ));
    };
    if unit.revision != expected_revision {
        return Err(OperationFailure::new(
            operation,
            "revision_mismatch",
            format!(
                "expected revision `{expected_revision}` but current revision is `{}`",
                unit.revision
            ),
            vec![id.to_owned()],
            "rebuild the request from the current revision",
        ));
    }
    Ok(unit)
}

fn require_projection_match(
    operation: &'static str,
    unit: &KnowledgeUnit,
    projection: &ProjectionRecord,
    expected_hash: &str,
) -> Result<(), OperationFailure> {
    if projection.metadata.knowledge_id != unit.metadata.id
        || projection.metadata.revision != unit.revision
        || projection.hash != expected_hash
    {
        return Err(OperationFailure::new(
            operation,
            "projection_mismatch",
            "Projection identity, revision, or content hash does not match the request",
            vec![unit.metadata.id.clone()],
            "regenerate the Projection and use its exact reported hash",
        ));
    }
    Ok(())
}

pub(crate) fn read_request<T>(path: &Path, operation: &'static str) -> Result<T, OperationFailure>
where
    T: serde::de::DeserializeOwned,
{
    let mut file = fs::File::open(path).map_err(|error| {
        OperationFailure::new(
            operation,
            "request_unreadable",
            error.to_string(),
            Vec::new(),
            "provide a readable versioned JSON request file",
        )
    })?;
    let mut bytes = Vec::new();
    Read::take(&mut file, (MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            OperationFailure::new(
                operation,
                "request_unreadable",
                error.to_string(),
                Vec::new(),
                "provide a readable versioned JSON request file",
            )
        })?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(OperationFailure::new(
            operation,
            "request_too_large",
            format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
            Vec::new(),
            "reduce the request to the required fields",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        OperationFailure::new(
            operation,
            "invalid_request",
            error.to_string(),
            Vec::new(),
            "repair the versioned JSON request",
        )
    })
}

pub(crate) fn require_schema(
    operation: &'static str,
    actual: &str,
    expected: &str,
    id: &str,
) -> Result<(), OperationFailure> {
    if actual == expected {
        Ok(())
    } else {
        Err(OperationFailure::new(
            operation,
            "unsupported_request_schema",
            format!("expected request schema `{expected}`"),
            vec![id.to_owned()],
            "regenerate the request using the current schema",
        ))
    }
}

pub(crate) fn normalize_markdown(
    markdown: &str,
    operation: &'static str,
    id: &str,
) -> Result<String, OperationFailure> {
    let normalized = markdown.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.trim().to_owned();
    if normalized.is_empty() {
        return Err(OperationFailure::new(
            operation,
            "empty_korean_markdown",
            "Korean review Markdown must not be empty",
            vec![id.to_owned()],
            "provide the exact Korean text to preserve for review",
        ));
    }
    if crate::check::body_has_forbidden_html(&normalized) {
        return Err(OperationFailure::new(
            operation,
            "raw_html_forbidden",
            "Korean review Markdown must not contain raw HTML blocks or comments",
            vec![id.to_owned()],
            "use visible Markdown or fenced code",
        ));
    }
    Ok(normalized)
}
