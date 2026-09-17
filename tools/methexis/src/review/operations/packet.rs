//! Review packet 생성과 atomic artifact publication을 담당한다.

use std::path::{Path, PathBuf};

use super::{
    foundation::{load_operation_foundation, require_projection_match, require_unit},
    request::{read_request, require_schema},
};
use crate::{
    model::KnowledgeUnit,
    review::{
        OperationFailure, OperationSuccess, ProjectionRecord, REVIEW_MANIFEST_SCHEMA,
        REVIEW_REQUEST_SCHEMA, ReviewManifest, ReviewRequest, SuccessInput,
        failure_from_diagnostic, hash_bytes, records, relative_path, semantic_hash, storage,
        success,
    },
};

pub(crate) fn build_review(
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
    let projection =
        records::parse_projection(&projection_path, repository_root).map_err(|diagnostic| {
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

/// 같은 unit과 Projection에 대해 build-review와 author-revision이 동일한
/// deterministic packet을 생성하고 publication 결과를 반환한다.
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
    let packet = records::render_review_packet(unit, projection);
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
    let status = storage::publish_artifact_directory(
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
