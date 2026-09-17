#[cfg(test)]
mod tests;

mod lock;
mod request;
mod result;
mod worktree;

use std::path::Path;

use lock::acquire_bootstrap_lock;
#[cfg(test)]
use result::Effect;
use result::{Effects, NextAction, ResultRecord, encode_failure};

use crate::{slice_contract, slice_worktree};

pub(crate) fn run(repository: &Path, source_path: &Path) -> Result<(), String> {
    let bytes = match request::read_contract(source_path) {
        Ok(bytes) => bytes,
        Err(error) => return Err(encode_failure(repository, None, error)?),
    };
    match prepare_bytes(repository, &bytes) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result)
                    .map_err(|error| format!("cannot encode Slice bootstrap result: {error}"))?
            );
            Ok(())
        },
        Err(error) => Err(encode_failure(repository, Some(&bytes), error)?),
    }
}

#[cfg(test)]
fn prepare(repository: &Path, source_path: &Path) -> Result<ResultRecord, String> {
    let bytes = request::read_contract(source_path)?;
    prepare_bytes(repository, &bytes)
}

fn prepare_bytes(repository: &Path, bytes: &[u8]) -> Result<ResultRecord, String> {
    let contract = request::parse_contract(bytes)?;
    request::validate_slice_name(&contract.slice)?;
    request::validate_base_ref_shape(&contract.base_ref)?;

    let repository = slice_worktree::repository_root(repository)?;
    let _lock = acquire_bootstrap_lock(&repository)?;
    let registered = slice_worktree::worktrees(&repository)?;
    let integration = worktree::unique_integration_worktree(&registered, &contract.base_ref)?;
    slice_worktree::ensure_clean(&integration.path, "integration worktree", "Slice bootstrap")?;
    slice_contract::validate_contract(&integration.path, &contract)?;

    let workspace = slice_worktree::workspace_root(&repository)?;
    let local = workspace.join(".local-exclude");
    let coordination = local.join("coordination");
    let worktrees_directory = local.join("worktrees");
    let contract_directory = coordination.join(&contract.slice);
    let contract_path = contract_directory.join("slice-contract.json");
    let worktree_path = worktrees_directory.join(&contract.slice);
    let branch_ref = worktree::branch_ref(&contract)?;
    slice_worktree::validate_branch_ref(&integration.path, &branch_ref)?;

    let existing_contract = request::read_optional_contract(&contract_path)?;
    let contract_prepared = existing_contract.as_deref() == Some(bytes);
    if existing_contract.is_some() && !contract_prepared {
        return Err(format!(
            "standard coordination contract {} contains different bytes",
            contract_path.display()
        ));
    }

    let existing_commit = match slice_worktree::existing_ref(&integration.path, &branch_ref)? {
        Some(slice_worktree::ExistingRef::Direct(commit)) => Some(commit),
        Some(slice_worktree::ExistingRef::Symbolic(target)) => {
            return Err(format!(
                "{branch_ref} is a symbolic ref to {target}; Slice bootstrap requires a direct branch ref"
            ));
        },
        None => None,
    };
    let existing_worktree = registered.iter().find(|worktree| {
        worktree.path == worktree_path || worktree.branch.as_deref() == Some(&branch_ref)
    });
    if !contract_prepared && (existing_commit.is_some() || existing_worktree.is_some()) {
        return Err(format!(
            "Slice ref or worktree already exists without the exact coordination contract {}",
            contract_path.display()
        ));
    }
    if let Some(actual) = existing_commit.as_deref()
        && actual != contract.base
    {
        return Err(format!(
            "{branch_ref} already points to {actual}, expected Slice base {}",
            contract.base
        ));
    }

    let integration_head = slice_worktree::resolve_commit(&integration.path, "HEAD")?;
    slice_worktree::expect_ref(&integration.path, &contract.base_ref, &integration_head)?;
    let exact_retry = contract_prepared && existing_commit.as_deref() == Some(&contract.base);
    if integration_head != contract.base && !exact_retry {
        return Err(format!(
            "Slice contract base {} is stale; current {} is {integration_head}",
            contract.base, contract.base_ref
        ));
    }

    if let Some(worktree) = existing_worktree {
        slice_worktree::validate_coordinates(
            worktree,
            &worktree_path,
            &branch_ref,
            &contract.base,
        )?;
        slice_worktree::ensure_clean(&worktree.path, "Slice worktree", "Slice bootstrap")?;
    } else if worktree::path_entry_exists(&worktree_path)? {
        return Err(format!(
            "unregistered Slice worktree path already exists at {}",
            worktree_path.display()
        ));
    }

    let checked_contracts =
        lock::verify_coordination_leases(&contract, &registered, &coordination, &contract_path)?;
    worktree::verify_registered_leases(
        &contract,
        &registered,
        &integration.path,
        &branch_ref,
        &worktree_path,
        checked_contracts,
    )?;

    for directory in [
        &local,
        &coordination,
        &worktrees_directory,
        &contract_directory,
    ] {
        bounded_file::ensure_directory(directory, "Slice bootstrap")?;
    }
    let contract_created = bounded_file::publish_new_or_exact(
        &contract_path,
        bytes,
        request::MAX_CONTRACT_BYTES,
        "Slice coordination contract",
    )?;

    let worktree_created = if existing_worktree.is_some() {
        false
    } else {
        slice_worktree::create(
            &integration.path,
            &worktree_path,
            &branch_ref,
            &contract.base,
            existing_commit.is_some(),
        )
        .map_err(|error| {
            format!(
                "{error}; exact contract is prepared at {} and the same command may be retried",
                contract_path.display()
            )
        })?;
        true
    };

    worktree::validate_prepared_worktree(
        &integration.path,
        &worktree_path,
        &branch_ref,
        &contract.base,
    )?;
    let binding =
        slice_contract::ensure_bound(&worktree_path, &contract_path).map_err(|error| {
            format!(
                "{error}; contract and worktree are prepared, so the same command may be retried"
            )
        })?;
    worktree::validate_prepared_worktree(
        &integration.path,
        &worktree_path,
        &branch_ref,
        &contract.base,
    )?;
    slice_contract::verify_bound_exact(&worktree_path, &contract_path)?;

    Ok(ResultRecord {
        schema: result::RESULT_SCHEMA,
        ok: true,
        slice: contract.slice,
        base: contract.base,
        base_ref: contract.base_ref,
        integration_worktree: integration.path.clone(),
        branch_ref,
        worktree_path: worktree_path.clone(),
        contract_path,
        binding_path: binding.binding_path,
        effects: Effects {
            contract: result::effect(contract_created),
            branch: result::effect(existing_commit.is_none()),
            worktree: result::effect(worktree_created),
            binding: result::effect(binding.created),
        },
        next_action: NextAction {
            cwd: worktree_path,
            argv: ["cargo", "xtask", "check", "slice-scope"],
        },
    })
}
