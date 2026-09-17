//! Review operation이 참조하는 foundation과 exact identity 검증을 담당한다.

use std::path::Path;

use crate::{
    check::{Foundation, load_foundation},
    model::KnowledgeUnit,
    review::{OperationFailure, ProjectionRecord},
};

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

pub(super) fn require_projection_match(
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
