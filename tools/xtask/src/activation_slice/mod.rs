mod model;
mod observation;
mod storage;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use model::{Effect, Effects, Request, ResultRecord};

use crate::{git, slice_contract, slice_worktree};

const DEVELOP_REF: &str = "refs/heads/develop";

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let bytes = match storage::read_request(request_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(serde_json::to_string_pretty(&observation::failure(
                repository, None, None, error,
            ))
            .map_err(|encode| format!("cannot encode activation Slice failure: {encode}"))?);
        },
    };
    let initial_base = slice_worktree::repository_root(repository)
        .and_then(|root| slice_worktree::resolve_commit(&root, "HEAD"))
        .ok();
    match prepare_bytes_with_post_binding(repository, &bytes, initial_base.as_deref(), |_| Ok(())) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result)
                    .map_err(|error| format!("cannot encode activation Slice result: {error}"))?
            );
            Ok(())
        },
        Err(error) => Err(serde_json::to_string_pretty(&observation::failure(
            repository,
            Some(&bytes),
            initial_base,
            error,
        ))
        .map_err(|encode| format!("cannot encode activation Slice failure: {encode}"))?),
    }
}

#[cfg(test)]
fn prepare(repository: &Path, request_path: &Path) -> Result<ResultRecord, String> {
    let bytes = storage::read_request(request_path)?;
    prepare_bytes_with_post_binding(repository, &bytes, None, |_| Ok(()))
}

#[cfg(test)]
fn prepare_with_post_binding(
    repository: &Path,
    request_path: &Path,
    post_binding: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<ResultRecord, String> {
    let bytes = storage::read_request(request_path)?;
    prepare_bytes_with_post_binding(repository, &bytes, None, post_binding)
}

fn prepare_bytes_with_post_binding(
    repository: &Path,
    bytes: &[u8],
    planned_base: Option<&str>,
    post_binding: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<ResultRecord, String> {
    let request: Request = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid activation Slice request: {error}"))?;
    request.validate()?;

    let repository = slice_worktree::repository_root(repository)?;
    slice_worktree::ensure_clean(
        &repository,
        "integration worktree",
        "activation Slice setup",
    )?;
    let branch = slice_worktree::current_branch_ref(&repository, "activation Slice setup")?;
    if branch != DEVELOP_REF {
        return Err(format!(
            "activation Slice setup must run from `{DEVELOP_REF}`, found `{branch}`"
        ));
    }
    let branch_ref = format!("refs/heads/slice/direct/{}", request.slice);
    validate_branch_ref(&repository, &branch_ref)?;

    let workspace = workspace_root(&repository)?;
    let local = workspace.join(".local-exclude");
    let coordination = local.join("coordination");
    let worktrees_directory = local.join("worktrees");
    for directory in [&local, &coordination, &worktrees_directory] {
        storage::ensure_directory(directory)?;
    }
    let contract_directory = coordination.join(&request.slice);
    let contract_path = contract_directory.join("slice-contract.json");
    let worktree_path = worktrees_directory.join(&request.slice);
    let current_base = match planned_base {
        Some(base) => base.to_owned(),
        None => slice_worktree::resolve_commit(&repository, "HEAD")?,
    };
    slice_worktree::expect_ref(&repository, DEVELOP_REF, &current_base)?;
    let existing_contract = storage::read_existing_contract(&contract_path)?;
    let base = match existing_contract.as_deref() {
        Some(bytes) => {
            let pinned = model::recover_contract_base(bytes, &request)?;
            validate_recovered_base(&repository, &pinned)?;
            pinned
        },
        None => current_base,
    };
    let contract_bytes = model::contract_bytes(&request, &base)?;

    let registered = slice_worktree::worktrees(&repository)?;
    let existing_worktree = registered.iter().find(|worktree| {
        worktree.path == worktree_path || worktree.branch.as_deref() == Some(&branch_ref)
    });
    let existing_ref = existing_ref(&repository, &branch_ref)?;
    let contract_exists = existing_contract.is_some();
    if !contract_exists && (existing_ref.is_some() || existing_worktree.is_some()) {
        return Err(format!(
            "Slice ref or worktree already exists without the exact activation contract {}",
            contract_path.display()
        ));
    }
    if let Some(actual) = existing_ref.as_deref()
        && actual != base
    {
        return Err(format!(
            "{branch_ref} already points to {actual}, expected setup base {base}"
        ));
    }
    if let Some(worktree) = existing_worktree {
        validate_worktree(worktree, &worktree_path, &branch_ref, &base)?;
        slice_worktree::ensure_clean(
            &worktree.path,
            "activation Slice worktree",
            "activation Slice setup",
        )?;
    } else if storage::path_entry_exists(&worktree_path)? {
        return Err(format!(
            "unregistered activation Slice worktree path already exists at {}",
            worktree_path.display()
        ));
    }

    storage::ensure_directory(&contract_directory)?;
    let contract_created = storage::publish_exact(&contract_path, &contract_bytes)?;

    let worktree_created = if existing_worktree.is_some() {
        false
    } else {
        create_worktree(
            &repository,
            &worktree_path,
            &branch_ref,
            &base,
            existing_ref.is_some(),
        )
        .map_err(|error| {
            format!(
                "{error}; exact contract is prepared at {} and the same request may be retried",
                contract_path.display()
            )
        })?;
        true
    };

    validate_prepared_worktree(&repository, &worktree_path, &branch_ref, &base)?;

    let binding = slice_contract::ensure_bound(&worktree_path, &contract_path).map_err(|error| {
        format!(
            "{error}; contract {} and worktree {} are prepared, so the same request may be retried",
            contract_path.display(),
            worktree_path.display()
        )
    })?;
    post_binding(&worktree_path)?;
    validate_prepared_worktree(&repository, &worktree_path, &branch_ref, &base)?;
    slice_contract::verify_bound_exact(&worktree_path, &contract_path)?;
    Ok(ResultRecord {
        schema: model::RESULT_SCHEMA,
        ok: true,
        slice: request.slice,
        base,
        branch_ref,
        worktree_path,
        contract_path,
        binding_path: binding.binding_path,
        effects: Effects {
            contract: effect(contract_created),
            branch: effect(existing_ref.is_none()),
            worktree: effect(worktree_created),
            binding: effect(binding.created),
        },
    })
}

fn validate_recovered_base(repository: &Path, base: &str) -> Result<(), String> {
    let resolved = slice_worktree::resolve_commit(repository, base)?;
    if resolved != base {
        return Err(format!(
            "existing activation Slice base `{base}` is not a full canonical commit ID"
        ));
    }
    if git::succeeds_in(
        repository,
        &["merge-base", "--is-ancestor", base, DEVELOP_REF],
        false,
    )? {
        Ok(())
    } else {
        Err(format!(
            "existing activation Slice base {base} no longer belongs to {DEVELOP_REF}"
        ))
    }
}

fn validate_branch_ref(repository: &Path, branch_ref: &str) -> Result<(), String> {
    let branch = branch_ref
        .strip_prefix("refs/heads/")
        .ok_or_else(|| format!("invalid Slice branch ref `{branch_ref}`"))?;
    if git::succeeds_in(repository, &["check-ref-format", "--branch", branch], false)? {
        Ok(())
    } else {
        Err(format!(
            "Slice name does not form a valid Git branch `{branch}`"
        ))
    }
}

fn validate_worktree(
    worktree: &slice_worktree::Worktree,
    expected_path: &Path,
    expected_branch: &str,
    expected_head: &str,
) -> Result<(), String> {
    if worktree.path == expected_path
        && worktree.branch.as_deref() == Some(expected_branch)
        && worktree.head == expected_head
    {
        Ok(())
    } else {
        Err(
            "existing Slice worktree does not match the requested path, branch, and base"
                .to_owned(),
        )
    }
}

fn validate_prepared_worktree(
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
        .ok_or_else(|| "prepared activation Slice worktree is no longer registered".to_owned())?;
    validate_worktree(worktree, expected_path, expected_branch, expected_head)?;
    slice_worktree::ensure_clean(
        &worktree.path,
        "activation Slice worktree",
        "activation Slice setup",
    )
}

fn workspace_root(repository: &Path) -> Result<PathBuf, String> {
    let common = git::output_in(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        false,
    )?;
    let common = PathBuf::from(common.trim());
    if common.file_name().and_then(|name| name.to_str()) != Some(".git") {
        return Err(format!(
            "unsupported common Git directory {}; expected a `.git` directory",
            common.display()
        ));
    }
    common
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "common Git directory has no workspace parent".to_owned())
}

fn existing_ref(repository: &Path, reference: &str) -> Result<Option<String>, String> {
    if git::succeeds_in(
        repository,
        &["show-ref", "--verify", "--quiet", reference],
        false,
    )? {
        slice_worktree::resolve_commit(repository, reference).map(Some)
    } else {
        Ok(None)
    }
}

fn create_worktree(
    repository: &Path,
    path: &Path,
    branch_ref: &str,
    base: &str,
    branch_exists: bool,
) -> Result<(), String> {
    let branch = branch_ref
        .strip_prefix("refs/heads/")
        .ok_or_else(|| format!("invalid Slice branch ref `{branch_ref}`"))?;
    let mut command = git::command_in(repository, false);
    command.args(["worktree", "add", "--quiet"]);
    if branch_exists {
        command.arg(path).arg(branch);
    } else {
        command.args(["-b", branch]).arg(path).arg(base);
    }
    let output = command
        .output()
        .map_err(|error| format!("cannot start git worktree add: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git worktree add failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn effect(created: bool) -> Effect {
    if created {
        Effect::Created
    } else {
        Effect::Reused
    }
}
