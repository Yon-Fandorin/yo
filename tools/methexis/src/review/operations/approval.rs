//! Projection 및 canonical approval 기록 orchestration을 담당한다.

use std::path::Path;

use super::{
    foundation::{load_operation_foundation, require_projection_match, require_unit},
    request::read_request,
};
use crate::review::{
    APPROVAL_REQUEST_SCHEMA, ApprovalInput, ApprovalRequest, CANONICAL_APPROVAL_REQUEST_SCHEMA,
    CanonicalApprovalInput, OperationFailure, OperationSuccess, SuccessInput,
    failure_from_diagnostic, hash_bytes, records, semantic_hash, storage, success,
    valid_review_time,
};

pub(crate) fn record_approval(
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
            let projection = records::parse_projection(&projection_path, repository_root).map_err(
                |diagnostic| {
                    failure_from_diagnostic(
                        OPERATION,
                        diagnostic,
                        "generate the matching Projection first",
                    )
                },
            )?;
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
    let bytes = records::render_approval(&request, projection.as_ref(), &request_hash);
    let hash = hash_bytes(&bytes);
    let target = repository_root
        .join("methexis/approvals")
        .join(format!("{}.yaml", request.knowledge_id()));
    let status = storage::publish_approval(
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
