mod git_state;
mod model;
mod storage;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use serde::Serialize;

use self::{
    git_state::{
        Worktree, acquire_cleanup_lock, current_branch_ref, delete_slice_ref_guarded, ensure_clean,
        expect_ref, find_accepted_commit, matching_patch_id, remove_worktree, repository_root,
        resolve_commit, worktrees,
    },
    model::{
        Effects, Plan, SCHEMA, identity, slice_ref_for, validate_plan_shape, validate_slice_name,
    },
};
use crate::{impact, slice_contract, slice_worktree};

pub(crate) fn plan(
    repository: &Path,
    slice: &str,
    output_path: Option<&Path>,
) -> Result<(), String> {
    let plan = build_plan(repository, slice)?;
    let mut bytes = serde_json::to_vec_pretty(&plan)
        .map_err(|error| format!("cannot encode Slice close plan: {error}"))?;
    bytes.push(b'\n');
    if let Some(path) = output_path {
        if path_within(path, &plan.worktree_path)? {
            return Err("store the Slice close plan outside the worktree it removes".to_owned());
        }
        let status = if storage::publish_plan(path, &bytes)? {
            "written"
        } else {
            "unchanged"
        };
        println!(
            "{{\"schema\":\"yo.slice-close-plan-publication/v1\",\"ok\":true,\"status\":{},\"slice\":{},\"plan_id\":{},\"path\":{}}}",
            json(&status)?,
            json(&plan.slice)?,
            json(&plan.plan_id)?,
            json(&path)?
        );
    } else {
        print!(
            "{}",
            String::from_utf8(bytes).expect("serialized Slice close plan is valid UTF-8")
        );
    }
    Ok(())
}

fn build_plan(repository: &Path, slice: &str) -> Result<Plan, String> {
    validate_slice_name(slice)?;
    let repository = repository_root(repository)?;
    ensure_clean(&repository, "integration worktree")?;
    let current_ref = current_branch_ref(&repository)?;
    let slice_ref = slice_ref_for(&current_ref, slice)?;
    let worktrees = worktrees(&repository)?;
    let candidates = worktrees
        .iter()
        .filter(|worktree| worktree.branch.as_deref() == Some(slice_ref.as_str()))
        .collect::<Vec<_>>();
    let target = match candidates.as_slice() {
        [target] => *target,
        [] => return Err(format!("no registered Slice worktree found for `{slice}`")),
        _ => {
            return Err(format!(
                "multiple registered Slice worktrees match `{slice}`"
            ));
        },
    };
    ensure_clean(&target.path, "Slice worktree")?;
    let bound = slice_contract::bound_slice(&target.path)?;
    if bound.slice != slice {
        return Err(format!(
            "Slice worktree binding names `{}`, not `{slice}`",
            bound.slice
        ));
    }
    if bound.base_ref != current_ref {
        return Err(format!(
            "run plan from `{}`; current integration ref is `{current_ref}`",
            bound.base_ref
        ));
    }

    let integration_head = resolve_commit(&repository, &current_ref)?;
    let slice_head = resolve_commit(&repository, &slice_ref)?;
    if target.head != slice_head {
        return Err(format!(
            "registered Slice worktree HEAD {} does not match {} {slice_head}",
            target.head, slice_ref
        ));
    }
    let (patch_id, accepted_commit) =
        find_accepted_commit(&repository, &integration_head, &bound.base, &slice_head)?;
    validate_accepted_commit(&repository, &current_ref, &accepted_commit)?;
    let remove_coordination_contract =
        bound.contract_path == standard_contract_path(&repository, slice)?;
    let mut plan = Plan {
        schema: SCHEMA.to_owned(),
        plan_id: String::new(),
        slice: slice.to_owned(),
        integration_ref: current_ref,
        integration_head: integration_head.clone(),
        accepted_commit,
        slice_ref,
        slice_head,
        slice_base: bound.base,
        patch_id,
        worktree_path: target.path.clone(),
        binding_path: bound.binding_path,
        contract_path: bound.contract_path,
        contract_id: bound.contract_id,
        effects: Effects::new(remove_coordination_contract),
    };
    plan.plan_id = identity(&plan)?;
    Ok(plan)
}

pub(crate) fn apply(repository: &Path, plan_path: &Path) -> Result<(), String> {
    apply_with_before_delete(repository, plan_path, || Ok(()))
}

fn apply_with_before_delete(
    repository: &Path,
    plan_path: &Path,
    before_delete: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let repository = repository_root(repository)?;
    let _lock = acquire_cleanup_lock(&repository)?;
    ensure_clean(&repository, "integration worktree")?;
    let bytes = storage::read_plan(plan_path)?;
    let plan: Plan = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Slice close plan {}: {error}", plan_path.display()))?;
    validate_plan_shape(&plan)?;
    validate_plan_effects(&repository, &plan)?;
    let actual_identity = identity(&plan)?;
    if plan.plan_id != actual_identity {
        return Err(format!(
            "Slice close plan identity mismatch: recorded {}, computed {actual_identity}",
            plan.plan_id
        ));
    }
    if path_within(plan_path, &plan.worktree_path)? {
        return Err("store the Slice close plan outside the worktree it removes".to_owned());
    }
    if current_branch_ref(&repository)? != plan.integration_ref {
        return Err(format!(
            "run apply from the planned integration ref `{}`",
            plan.integration_ref
        ));
    }
    expect_ref(&repository, &plan.integration_ref, &plan.integration_head)?;
    expect_ref(&repository, &plan.slice_ref, &plan.slice_head)?;
    validate_accepted_commit(&repository, &plan.integration_ref, &plan.accepted_commit)?;
    let current_patch = matching_patch_id(
        &repository,
        &plan.slice_base,
        &plan.slice_head,
        &plan.accepted_commit,
    )?;
    if current_patch != plan.patch_id {
        return Err("Slice patch identity changed after planning".to_owned());
    }

    let registered = worktrees(&repository)?;
    let at_path = registered
        .iter()
        .find(|worktree| worktree.path == plan.worktree_path);
    let on_branch = registered
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(plan.slice_ref.as_str()));
    let worktree_was_present = at_path.is_some();
    match (at_path, on_branch) {
        (Some(worktree), Some(branch_worktree)) if worktree == branch_worktree => {
            validate_registered_worktree(&plan, worktree)?;
            ensure_clean(&worktree.path, "Slice worktree")?;
            let bound = slice_contract::bound_slice(&worktree.path)?;
            validate_bound_plan(&plan, &bound)?;
            let refreshed = worktrees(&repository)?;
            let refreshed = refreshed
                .iter()
                .find(|candidate| candidate.path == plan.worktree_path)
                .ok_or_else(|| "planned Slice worktree disappeared during apply".to_owned())?;
            validate_registered_worktree(&plan, refreshed)?;
            remove_worktree(&repository, &plan.worktree_path)?;
        },
        (Some(_), _) => {
            return Err("planned worktree is still registered with different state".to_owned());
        },
        (None, Some(_)) => {
            return Err("planned Slice branch moved to another registered worktree".to_owned());
        },
        (None, None) => {
            if path_exists(&plan.worktree_path)? || path_exists(&plan.binding_path)? {
                return Err(
                    "planned worktree is not registered but its path or binding still exists"
                        .to_owned(),
                );
            }
        },
    }

    if plan.effects.remove_coordination_contract {
        storage::remove_coordination_contract(&plan.contract_path, &plan.contract_id)?;
    }

    before_delete()?;
    if let Err(error) = delete_slice_ref_guarded(
        &repository,
        &plan.integration_ref,
        &plan.integration_head,
        &plan.slice_ref,
        &plan.slice_head,
    ) {
        let state = if worktree_was_present && plan.effects.remove_coordination_contract {
            "the planned worktree, binding, and standard coordination contract were removed"
        } else if worktree_was_present {
            "the planned worktree and binding were removed"
        } else {
            "the planned worktree and binding were already absent; any planned standard coordination contract was reconciled"
        };
        return Err(format!(
            "{error}; {state}, but the exact Slice ref {} at {} was preserved. Inspect the current integration ref and preserved Slice ref, then perform a separately verified branch cleanup",
            plan.slice_ref, plan.slice_head
        ));
    }
    println!(
        "{{\"schema\":\"yo.slice-close-apply/v1\",\"ok\":true,\"slice\":{},\"plan_id\":{}}}",
        json(&plan.slice)?,
        json(&plan.plan_id)?
    );
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn validate_registered_worktree(plan: &Plan, worktree: &Worktree) -> Result<(), String> {
    if worktree.path != plan.worktree_path
        || worktree.head != plan.slice_head
        || worktree.branch.as_deref() != Some(plan.slice_ref.as_str())
    {
        return Err("registered Slice worktree no longer matches the plan".to_owned());
    }
    Ok(())
}

fn validate_bound_plan(plan: &Plan, bound: &slice_contract::BoundSlice) -> Result<(), String> {
    if bound.slice != plan.slice
        || bound.base != plan.slice_base
        || bound.base_ref != plan.integration_ref
        || bound.binding_path != plan.binding_path
        || bound.contract_path != plan.contract_path
        || bound.contract_id != plan.contract_id
    {
        return Err("Slice binding or contract changed after planning".to_owned());
    }
    Ok(())
}

fn validate_plan_effects(repository: &Path, plan: &Plan) -> Result<(), String> {
    let expected = plan.contract_path == standard_contract_path(repository, &plan.slice)?;
    if plan.effects.remove_coordination_contract == expected {
        Ok(())
    } else {
        Err("Slice close plan coordination-contract effect does not match its bounded standard path".to_owned())
    }
}

fn standard_contract_path(repository: &Path, slice: &str) -> Result<PathBuf, String> {
    Ok(slice_worktree::workspace_root(repository)?
        .join(".local-exclude/coordination")
        .join(slice)
        .join("slice-contract.json"))
}

fn validate_accepted_commit(
    repository: &Path,
    integration_ref: &str,
    accepted_commit: &str,
) -> Result<(), String> {
    let branch = integration_ref
        .strip_prefix("refs/heads/")
        .ok_or_else(|| format!("unsupported integration ref `{integration_ref}`"))?;
    impact::slice_review::check_commit(repository, accepted_commit, branch).map_err(|error| {
        format!("accepted commit {accepted_commit} has invalid review evidence: {error}")
    })
}

fn path_within(path: &Path, directory: &Path) -> Result<bool, String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = std::fs::canonicalize(parent)
        .map_err(|error| format!("cannot resolve plan parent {}: {error}", parent.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("Slice close plan path {} has no file name", path.display()))?;
    let path = parent.join(name);
    let directory = if directory.exists() {
        std::fs::canonicalize(directory).map_err(|error| {
            format!(
                "cannot resolve planned worktree {}: {error}",
                directory.display()
            )
        })?
    } else if directory.is_absolute() {
        directory.to_owned()
    } else {
        return Err("planned worktree path must be absolute".to_owned());
    };
    Ok(path.starts_with(directory))
}

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| format!("cannot encode result: {error}"))
}
