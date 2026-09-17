//! Review Projection 생성과 tracked publication을 담당한다.

use std::path::Path;

use super::{
    foundation::{load_operation_foundation, require_unit},
    request::{normalize_markdown, read_request, require_schema},
};
use crate::review::{
    OperationFailure, OperationSuccess, PROJECTION_REQUEST_SCHEMA, ProjectionRequest, SuccessInput,
    hash_bytes, records, storage, success,
};

pub(crate) fn generate_projection(
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
        records::projection_input_hash(&request.knowledge_id, &request.expected_revision, &korean);
    let bytes = records::render_projection(unit, &request_hash, &korean);
    let hash = hash_bytes(&bytes);
    let target = repository_root
        .join("methexis/review-projections")
        .join(format!("{}.md", request.knowledge_id));
    let status = storage::publish_tracked(
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
