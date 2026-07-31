//! Read-only validation for one exact staged Checkpoint activation transition.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{
    ActiveCheckpoint, OperationFailure, StagedFallback, StagedTransition, git, hash_bytes,
    records::{build_active, parse_active_bytes, parse_checkpoint_bytes, read_active},
    validation,
};
use crate::{check::artifacts, source};

pub(super) const OPERATION: &str = "check_staged_activation";
const ACTIVE_PATH: &str = "methexis/active-checkpoint.yaml";

struct CandidatePath {
    status: char,
    path: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ProspectiveActivation {
    schema: &'static str,
    pub(crate) ok: bool,
    operation: &'static str,
    authority: &'static str,
    trusted_commit: String,
    current_active_hash: Option<String>,
    checkpoint_id: String,
    checkpoint_hash: String,
    checkpoint: &'static str,
    affected_ids: Vec<String>,
    staged_paths: Vec<String>,
    next_actions: Vec<String>,
}

pub(super) fn check_staged(
    repository_root: &std::path::Path,
    trusted_ref: &str,
) -> Result<StagedTransition, OperationFailure> {
    let index = git::capture_index(repository_root, OPERATION)?;
    let entries = git::staged_entries(repository_root, &index, OPERATION)?;
    if !contains_staged_active_record(&entries) {
        return Ok(StagedTransition::Ordinary(StagedFallback { index }));
    }
    let entries = decode_candidate_paths(entries)?;
    validate_candidate_paths(&entries)?;

    let snapshot = git::resolve(repository_root, trusted_ref, OPERATION)?;
    let current_active_path = snapshot.root.join(ACTIVE_PATH);
    let current_active_hash = if current_active_path.exists() {
        let (_, bytes) = read_active(&current_active_path, OPERATION)?;
        Some(hash_bytes(&bytes))
    } else {
        None
    };

    let active_bytes = git::read_index_blob(repository_root, &index, ACTIVE_PATH, OPERATION)?;
    let active = parse_active_bytes(&active_bytes, OPERATION)?;
    if active.replaces_active_hash != current_active_hash {
        return Err(failure(
            Some(snapshot.commit.clone()),
            "active_checkpoint_compare_and_swap_mismatch",
            "staged active record does not name the exact current trusted active-record hash",
            vec![active.checkpoint_id],
            "regenerate the activation proposal from the current trusted integration",
        ));
    }

    let checkpoint_path = checkpoint_path(&active.checkpoint_id);
    let checkpoint_bytes =
        git::read_index_blob(repository_root, &index, &checkpoint_path, OPERATION)?;
    let checkpoint = parse_checkpoint_bytes(&checkpoint_bytes, OPERATION)?;
    let checkpoint_hash = hash_bytes(&checkpoint_bytes);
    if checkpoint.checkpoint_id != active.checkpoint_id
        || checkpoint_hash != active.checkpoint_hash
        || checkpoint.trusted_commit != active.trusted_commit
    {
        return Err(failure(
            Some(snapshot.commit.clone()),
            "active_checkpoint_mismatch",
            "staged active record does not match the exact staged immutable Checkpoint",
            vec![active.checkpoint_id],
            "regenerate the activation proposal from the staged Checkpoint",
        ));
    }
    if checkpoint.trusted_commit != snapshot.commit {
        return Err(failure(
            Some(snapshot.commit.clone()),
            "checkpoint_trust_mismatch",
            "staged Checkpoint was not created from the current trusted integration",
            vec![checkpoint.checkpoint_id],
            "recreate the Checkpoint from the current trusted integration",
        ));
    }
    validation::verify_lineage(&snapshot, &checkpoint, &checkpoint_bytes, OPERATION)?;

    let (_, expected_active_bytes, _) = build_active(
        &checkpoint,
        &checkpoint_hash,
        current_active_hash.as_deref(),
    )?;
    if active_bytes != expected_active_bytes {
        return Err(failure(
            Some(snapshot.commit.clone()),
            "active_checkpoint_lineage_mismatch",
            "staged active record is not the canonical transition from current trusted authority",
            vec![checkpoint.checkpoint_id],
            "regenerate the activation proposal with propose-activation",
        ));
    }

    let selected = checkpoint
        .units
        .iter()
        .map(|unit| unit.id.clone())
        .collect::<BTreeSet<_>>();
    let foundation = crate::check::load_foundation(&snapshot.root).map_err(|diagnostics| {
        let message = diagnostics.first().map_or_else(
            || "trusted foundation is invalid".to_owned(),
            |item| item.message.clone(),
        );
        failure(
            Some(snapshot.commit.clone()),
            "trusted_foundation_invalid",
            message,
            diagnostics
                .into_iter()
                .flat_map(|item| item.affected_ids)
                .collect(),
            "repair and review the trusted foundation",
        )
    })?;
    let (working_sources, captures) =
        source::load_captured(repository_root).map_err(|diagnostics| {
            let message = diagnostics.first().map_or_else(
                || "working Source records are invalid".to_owned(),
                |item| item.message.clone(),
            );
            failure(
                Some(snapshot.commit.clone()),
                "source_records_invalid",
                message,
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
                    Some(snapshot.commit.clone()),
                    error.code,
                    error.message,
                    error.affected_ids,
                    "repair the Source observation and retry",
                )
            },
        )?;
    source_evaluation.guard.add_record_captures(captures);
    let degraded = source_evaluation
        .units
        .iter()
        .filter(|(_, state)| state.eligibility != source::Eligibility::Active)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    if !degraded.is_empty() {
        return Err(failure(
            Some(snapshot.commit.clone()),
            "prospective_checkpoint_degraded",
            "staged activation contains selected knowledge that is not currently fresh",
            degraded,
            "repair Source freshness before activating the Checkpoint",
        ));
    }

    let candidate_active = ActiveCheckpoint {
        id: checkpoint.checkpoint_id.clone(),
        hash: checkpoint_hash.clone(),
        active_record_hash: hash_bytes(&active_bytes),
        authority_basis_commit: checkpoint.trusted_commit.clone(),
    };
    let artifact_bytes = artifacts::TRACKED_ARTIFACTS
        .iter()
        .map(|path| {
            git::read_index_blob(repository_root, &index, path, OPERATION)
                .map(|bytes| ((*path).to_owned(), bytes))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let diagnostics = artifacts::validate_candidate(&artifact_bytes, &candidate_active);
    if !diagnostics.is_empty() {
        return Err(failure(
            Some(snapshot.commit.clone()),
            diagnostics[0].code.clone(),
            diagnostics[0].message.clone(),
            diagnostics
                .into_iter()
                .flat_map(|item| item.affected_ids)
                .collect(),
            "refresh every registered tracked artifact from the staged Checkpoint",
        ));
    }

    source::final_revalidate(repository_root, &source_evaluation.guard).map_err(|error| {
        failure(
            Some(snapshot.commit.clone()),
            error.code,
            error.message,
            error.affected_ids,
            "retry after the Source files stop changing",
        )
    })?;
    git::ensure_index_unchanged(repository_root, &index, OPERATION)?;
    git::ensure_ref_unchanged(repository_root, trusted_ref, &snapshot.commit, OPERATION)?;

    Ok(StagedTransition::Prospective(ProspectiveActivation {
        schema: "methexis.prospective-activation/v1alpha1",
        ok: true,
        operation: OPERATION,
        authority: "prospective",
        trusted_commit: snapshot.commit.clone(),
        current_active_hash,
        checkpoint_id: checkpoint.checkpoint_id,
        checkpoint_hash,
        checkpoint: source_evaluation.checkpoint,
        affected_ids: checkpoint.units.into_iter().map(|unit| unit.id).collect(),
        staged_paths: entries.into_iter().map(|entry| entry.path).collect(),
        next_actions: vec![
            "review and integrate this exact staged transition, then run ordinary `methexis check`"
                .to_owned(),
        ],
    }))
}

fn contains_staged_active_record(entries: &[git::StagedEntry]) -> bool {
    entries
        .iter()
        .any(|entry| entry.path == ACTIVE_PATH.as_bytes())
}

fn decode_candidate_paths(
    entries: Vec<git::StagedEntry>,
) -> Result<Vec<CandidatePath>, OperationFailure> {
    entries
        .into_iter()
        .map(|entry| {
            String::from_utf8(entry.path)
                .map(|path| CandidatePath {
                    status: entry.status,
                    path,
                })
                .map_err(|error| {
                    failure(
                        None,
                        "invalid_git_path",
                        error.to_string(),
                        Vec::new(),
                        "use UTF-8 repository paths for activation records",
                    )
                })
        })
        .collect()
}

fn validate_candidate_paths(entries: &[CandidatePath]) -> Result<(), OperationFailure> {
    let checkpoint_paths = entries
        .iter()
        .filter(|entry| {
            entry.path.starts_with("methexis/checkpoints/") && entry.path.ends_with(".yaml")
        })
        .collect::<Vec<_>>();
    let allowed = |path: &str| {
        path == ACTIVE_PATH
            || checkpoint_paths
                .first()
                .is_some_and(|entry| entry.path == path)
            || artifacts::TRACKED_ARTIFACTS.contains(&path)
    };
    if checkpoint_paths.len() != 1
        || entries.len() != artifacts::TRACKED_ARTIFACTS.len() + 2
        || entries.iter().any(|entry| !allowed(&entry.path))
        || entries.iter().any(|entry| entry.status == 'D')
        || entries
            .iter()
            .find(|entry| entry.path == ACTIVE_PATH)
            .is_none_or(|entry| !matches!(entry.status, 'A' | 'M'))
        || checkpoint_paths[0].status != 'A'
    {
        return Err(failure(
            None,
            "invalid_activation_candidate_paths",
            "staged transition must contain exactly one new Checkpoint, the active record, and every registered tracked artifact",
            entries.iter().map(|entry| entry.path.clone()).collect(),
            "separate unrelated changes and stage the complete activation transition",
        ));
    }
    Ok(())
}

fn checkpoint_path(id: &str) -> String {
    format!(
        "methexis/checkpoints/{}.yaml",
        id.strip_prefix("sha256:")
            .expect("validated active CheckpointId has a sha256 prefix")
    )
}

fn failure(
    trusted_commit: Option<String>,
    code: impl Into<String>,
    message: impl Into<String>,
    affected_ids: Vec<String>,
    next_action: impl Into<String>,
) -> OperationFailure {
    OperationFailure::new(
        OPERATION,
        trusted_commit,
        code,
        message,
        affected_ids,
        next_action,
    )
}

#[cfg(test)]
mod tests {
    use super::{contains_staged_active_record, git};

    // 파일시스템이 invalid UTF-8 이름을 만들 수 없는 host에서도 unrelated raw Git path는
    // active record로 오인하지 않아 staged activation이 ordinary fallback을 유지한다.
    #[test]
    fn unrelated_non_utf8_raw_path_does_not_select_staged_activation() {
        let entries = [git::StagedEntry {
            status: 'A',
            path: b"unrelated-\xff".to_vec(),
        }];

        assert!(!contains_staged_active_record(&entries));
    }
}
