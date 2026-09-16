use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use super::{
    model::{EffectiveValidation, ReviewLineage, ScanBudget, SliceState},
    scan::{collect_json, resolve_status_path},
};
use crate::{bounded_file, git, review_protocol, slice_contract, slice_worktree};

pub(super) fn locate(repository: &Path, slice: &str) -> Result<SliceState, String> {
    validate_slice_name(slice)?;
    let mut matches = Vec::new();
    for worktree in slice_worktree::worktrees(repository)? {
        let Some(branch) = worktree.branch.as_deref() else {
            continue;
        };
        if !branch_names_slice(branch, slice) {
            continue;
        }
        let bound = slice_contract::bound_slice(&worktree.path)?;
        if bound.slice == slice {
            matches.push((worktree, bound));
        }
    }
    let (worktree, bound) = match matches.as_slice() {
        [(worktree, bound)] => (worktree, bound),
        [] => return Err(format!("no registered Slice worktree found for `{slice}`")),
        _ => {
            return Err(format!(
                "multiple registered Slice worktrees match `{slice}`"
            ));
        },
    };
    let clean = git::output_bytes_in(
        &worktree.path,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        false,
    )?
    .is_empty();
    Ok(SliceState {
        worktree: worktree.path.clone(),
        branch: worktree
            .branch
            .clone()
            .expect("matched Slice worktree has a branch"),
        head: worktree.head.clone(),
        bound: bound.clone(),
        clean,
    })
}

pub(super) fn branch_names_slice(branch: &str, slice: &str) -> bool {
    branch == format!("refs/heads/slice/direct/{slice}")
        || branch
            .strip_prefix("refs/heads/slice/")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(wave, name)| !wave.is_empty() && name == slice)
}

pub(super) fn validate_slice_name(slice: &str) -> Result<(), String> {
    if slice.is_empty()
        || slice != slice.trim()
        || slice.contains('/')
        || matches!(slice, "." | "..")
    {
        Err("Slice name must be one non-empty branch segment".to_owned())
    } else {
        Ok(())
    }
}

pub(super) fn scan_review_lineage(
    state: &SliceState,
    workspace: &Path,
    budget: &mut ScanBudget,
) -> Result<ReviewLineage, String> {
    let mut files = Vec::new();
    for root in [
        state.worktree.join(".local-exclude/methexis"),
        workspace.join(".local-exclude/methexis"),
    ] {
        if root.exists() {
            collect_json(&root, 0, &mut files, budget)?;
        }
    }
    files.sort();
    files.dedup();
    let mut candidates = BTreeSet::new();
    let mut review_ids = BTreeMap::<String, BTreeSet<String>>::new();
    let mut current_review_ids = BTreeSet::new();
    let mut current_validations = BTreeMap::<String, EffectiveValidation>::new();
    let mut broken = false;
    for path in files {
        if path.file_name().and_then(|name| name.to_str()) != Some("manifest.json") {
            continue;
        }
        let bytes = bounded_file::read_regular(&path, JSON_LIMIT, "published review manifest")?;
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        if value
            .pointer("/plan/slice_contract/hash")
            .and_then(serde_json::Value::as_str)
            != Some(state.bound.contract_id.as_str())
        {
            continue;
        }
        let candidate = value
            .pointer("/plan/candidate_commit")
            .or_else(|| value.pointer("/plan/replacement_candidate_commit"))
            .and_then(serde_json::Value::as_str);
        let Some(candidate) = candidate else { continue };
        if !is_commit(candidate) {
            broken = true;
            continue;
        }
        candidates.insert(candidate.to_owned());
        if let Some(review_id) = value
            .get("review_id")
            .or_else(|| value.get("review_delta_id"))
            .and_then(serde_json::Value::as_str)
        {
            review_ids
                .entry(candidate.to_owned())
                .or_default()
                .insert(review_id.to_owned());
            if candidate == state.head {
                current_review_ids.insert(review_id.to_owned());
                for validation in manifest_validations(&value, &state.worktree)? {
                    if current_validations
                        .insert(validation.name.clone(), validation)
                        .is_some()
                    {
                        broken = true;
                    }
                }
            }
        }
        if !git::trusted_succeeds_in(
            &state.worktree,
            &["merge-base", "--is-ancestor", candidate, &state.head],
        )? {
            broken = true;
        }
    }
    let mut latest = None;
    let mut smallest_distance = u64::MAX;
    for candidate in &candidates {
        if !git::trusted_succeeds_in(
            &state.worktree,
            &["merge-base", "--is-ancestor", candidate, &state.head],
        )? {
            continue;
        }
        let count = git::trusted_output_in(
            &state.worktree,
            &[
                "rev-list",
                "--count",
                &format!("{candidate}..{}", state.head),
            ],
        )?;
        let distance = count
            .trim()
            .parse::<u64>()
            .map_err(|error| format!("Git returned invalid review distance: {error}"))?;
        if distance < smallest_distance {
            smallest_distance = distance;
            latest = Some(candidate.clone());
        }
    }
    let latest_review_ids = latest
        .as_ref()
        .and_then(|candidate| review_ids.get(candidate))
        .cloned()
        .unwrap_or_default();
    Ok(ReviewLineage {
        packets: candidates.len(),
        latest_candidate: latest,
        status: if broken { "broken" } else { "preserved" },
        current_review_ids,
        latest_review_ids,
        current_validations: current_validations.into_values().collect(),
    })
}

fn manifest_validations(
    manifest: &serde_json::Value,
    repository: &Path,
) -> Result<Vec<EffectiveValidation>, String> {
    let mut result = Vec::new();
    for (pointer, reused) in [
        ("/inputs/validation_evidence", false),
        ("/inputs/reused_validation_evidence", true),
        ("/inputs/affected_validation_evidence", false),
    ] {
        let Some(entries) = manifest
            .pointer(pointer)
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for entry in entries {
            let Some(name) = entry.get("name").and_then(serde_json::Value::as_str) else {
                return Err("review manifest validation evidence has no name".to_owned());
            };
            let Some(path) = entry
                .pointer("/artifact/path")
                .and_then(serde_json::Value::as_str)
            else {
                return Err("review manifest validation evidence has no artifact path".to_owned());
            };
            let Some(hash) = entry
                .pointer("/artifact/hash")
                .and_then(serde_json::Value::as_str)
            else {
                return Err("review manifest validation evidence has no artifact hash".to_owned());
            };
            result.push(EffectiveValidation {
                name: name.to_owned(),
                path: resolve_status_path(repository, path),
                hash: hash.to_owned(),
                reused,
            });
        }
    }
    Ok(result)
}

fn is_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
