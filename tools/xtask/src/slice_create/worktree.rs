use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use crate::{slice_contract, slice_worktree};

pub(super) fn unique_integration_worktree<'a>(
    worktrees: &'a [slice_worktree::Worktree],
    base_ref: &str,
) -> Result<&'a slice_worktree::Worktree, String> {
    let matches = worktrees
        .iter()
        .filter(|worktree| worktree.branch.as_deref() == Some(base_ref))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [integration] => Ok(integration),
        [] => Err(format!(
            "no registered integration worktree is attached to `{base_ref}`"
        )),
        _ => Err(format!(
            "multiple registered integration worktrees are attached to `{base_ref}`"
        )),
    }
}

pub(super) fn branch_ref(contract: &slice_contract::SliceContract) -> Result<String, String> {
    if contract.base_ref == "refs/heads/develop" {
        return Ok(format!("refs/heads/slice/direct/{}", contract.slice));
    }
    let wave = contract
        .base_ref
        .strip_prefix("refs/heads/wave/")
        .filter(|wave| !wave.is_empty())
        .ok_or_else(|| format!("invalid Wave integration ref `{}`", contract.base_ref))?;
    Ok(format!("refs/heads/slice/{wave}/{}", contract.slice))
}

pub(super) fn verify_registered_leases(
    contract: &slice_contract::SliceContract,
    registered: &[slice_worktree::Worktree],
    integration_path: &Path,
    target_branch: &str,
    target_path: &Path,
    checked_contracts: BTreeSet<PathBuf>,
) -> Result<(), String> {
    let mut checked_contracts = checked_contracts;
    for worktree in registered {
        if worktree.path == integration_path
            || worktree.path == target_path
            || worktree.branch.as_deref() == Some(target_branch)
        {
            continue;
        }
        let Some(branch) = worktree.branch.as_deref() else {
            continue;
        };
        if !branch.starts_with("refs/heads/slice/") && !branch.starts_with("refs/heads/task/") {
            continue;
        }
        let active = slice_contract::active_contract(&worktree.path).map_err(|error| {
            format!(
                "cannot verify active leases for worktree {}: {error}",
                worktree.path.display()
            )
        })?;
        let active_path = fs::canonicalize(&active.contract_path).map_err(|error| {
            format!(
                "cannot resolve active contract {}: {error}",
                active.contract_path.display()
            )
        })?;
        if !checked_contracts.insert(active_path) {
            continue;
        }
        slice_contract::ensure_lease_compatible(contract, &active.contract).map_err(|error| {
            format!(
                "{error}\nactive contract: {}",
                active.contract_path.display()
            )
        })?;
    }
    Ok(())
}

pub(super) fn validate_prepared_worktree(
    repository: &Path,
    expected_path: &Path,
    expected_branch: &str,
    expected_head: &str,
) -> Result<(), String> {
    slice_worktree::expect_ref(repository, expected_branch, expected_head)?;
    let registered = slice_worktree::worktrees(repository)?;
    let worktree = registered
        .iter()
        .find(|worktree| {
            worktree.path == expected_path || worktree.branch.as_deref() == Some(expected_branch)
        })
        .ok_or_else(|| "prepared Slice worktree is no longer registered".to_owned())?;
    slice_worktree::validate_coordinates(worktree, expected_path, expected_branch, expected_head)?;
    slice_worktree::ensure_clean(&worktree.path, "Slice worktree", "Slice bootstrap")
}

pub(super) fn path_entry_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}
