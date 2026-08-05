//! Shared validated prospective Checkpoint to ContextAuthority projection.

use std::{collections::BTreeSet, path::Path};

use super::{CheckpointRecord, ContextAuthority, OperationFailure, git, hash_bytes, validation};
use crate::{check::load_foundation, review::validate_records, source};

pub(super) fn build(
    repository_root: &Path,
    snapshot: &git::TrustedSnapshot,
    checkpoint: &CheckpointRecord,
    checkpoint_bytes: &[u8],
    active_bytes: &[u8],
    operation: &'static str,
) -> Result<ContextAuthority, OperationFailure> {
    validation::verify_lineage(snapshot, checkpoint, checkpoint_bytes, operation)?;
    let foundation = load_foundation(&snapshot.root).map_err(|diagnostics| {
        failure(
            operation,
            &snapshot.commit,
            "trusted_foundation_invalid",
            diagnostics.first().map_or_else(
                || "trusted foundation is invalid".to_owned(),
                |item| item.message.clone(),
            ),
            diagnostics
                .into_iter()
                .flat_map(|item| item.affected_ids)
                .collect(),
            "repair and review the trusted foundation",
        )
    })?;
    let review = validate_records(&snapshot.root, &foundation);
    if !review.diagnostics.is_empty() {
        return Err(failure(
            operation,
            &snapshot.commit,
            "trusted_review_invalid",
            review.diagnostics[0].message.clone(),
            review
                .diagnostics
                .into_iter()
                .flat_map(|item| item.affected_ids)
                .collect(),
            "repair trusted review records",
        ));
    }
    let approvals = foundation
        .units
        .iter()
        .filter(|unit| {
            review
                .states
                .get(&unit.metadata.id)
                .is_some_and(|state| state.evidence == "matching_proposal")
        })
        .map(|unit| (unit.metadata.id.clone(), unit.revision.clone()))
        .collect();
    validation::validate_integrated(
        &snapshot.commit,
        &foundation,
        &approvals,
        checkpoint,
        operation,
    )?;
    let selected = checkpoint
        .units
        .iter()
        .map(|unit| unit.id.clone())
        .collect::<BTreeSet<_>>();
    let (working_sources, captures) =
        source::load_captured(repository_root).map_err(|diagnostics| {
            failure(
                operation,
                &snapshot.commit,
                "source_records_invalid",
                diagnostics.first().map_or_else(
                    || "working Source records are invalid".to_owned(),
                    |item| item.message.clone(),
                ),
                diagnostics
                    .into_iter()
                    .flat_map(|item| item.affected_ids)
                    .collect(),
                "repair the Source records and retry",
            )
        })?;
    let mut source_evaluation =
        source::evaluate(repository_root, &foundation, &working_sources, &selected).map_err(
            |error| {
                failure(
                    operation,
                    &snapshot.commit,
                    error.code,
                    error.message,
                    error.affected_ids,
                    "repair the Source observation and retry",
                )
            },
        )?;
    source_evaluation.guard.add_record_captures(captures);
    let active = source_evaluation
        .units
        .iter()
        .filter(|(_, state)| state.eligibility == source::Eligibility::Active)
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    if active != selected {
        return Err(failure(
            operation,
            &snapshot.commit,
            "prospective_checkpoint_degraded",
            "prospective Checkpoint contains selected knowledge that is not currently fresh",
            selected.difference(&active).cloned().collect(),
            "repair Source freshness before using the prospective Checkpoint",
        ));
    }
    let unit_paths = foundation
        .units
        .iter()
        .map(|unit| {
            (
                unit.metadata.id.clone(),
                unit.path
                    .strip_prefix(&snapshot.root)
                    .unwrap_or(&unit.path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
        })
        .collect();
    Ok(ContextAuthority {
        trusted_commit: snapshot.commit.clone(),
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        checkpoint_hash: hash_bytes(checkpoint_bytes),
        authority_basis_commit: checkpoint.trusted_commit.clone(),
        foundation,
        unit_paths,
        active,
        freshness: source_evaluation.units,
        approval_evidence: review.evidence,
        freshness_guard: source_evaluation.guard,
        active_record_hash: hash_bytes(active_bytes),
    })
}

pub(super) fn final_revalidate(
    repository_root: &Path,
    authority: &ContextAuthority,
    trusted_ref: &str,
    operation: &'static str,
) -> Result<(), OperationFailure> {
    source::final_revalidate(repository_root, &authority.freshness_guard).map_err(|error| {
        failure(
            operation,
            &authority.trusted_commit,
            error.code,
            error.message,
            error.affected_ids,
            "retry after the Source files stop changing",
        )
    })?;
    git::ensure_ref_unchanged(
        repository_root,
        trusted_ref,
        &authority.trusted_commit,
        operation,
    )
}

fn failure(
    operation: &'static str,
    commit: &str,
    code: impl Into<String>,
    message: impl Into<String>,
    affected_ids: Vec<String>,
    next_action: impl Into<String>,
) -> OperationFailure {
    OperationFailure::new(
        operation,
        Some(commit.to_owned()),
        code,
        message,
        affected_ids,
        next_action,
    )
}
