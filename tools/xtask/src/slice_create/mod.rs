#[cfg(test)]
mod tests;

use std::{
    collections::BTreeSet,
    fs::File,
    path::{Path, PathBuf},
};

use rustix::fs::{FileType, FlockOperation, Mode, OFlags, flock, fstat, open, openat};
use serde::Serialize;

use crate::{bounded_file, git, slice_contract, slice_worktree};

const RESULT_SCHEMA: &str = "yo.slice-bootstrap/v1alpha1";
const MAX_CONTRACT_BYTES: usize = 64 * 1024;

#[derive(Debug, Serialize)]
pub(crate) struct ResultRecord {
    schema: &'static str,
    ok: bool,
    slice: String,
    base: String,
    base_ref: String,
    integration_worktree: PathBuf,
    branch_ref: String,
    worktree_path: PathBuf,
    contract_path: PathBuf,
    binding_path: PathBuf,
    effects: Effects,
    next_action: NextAction,
}

#[derive(Debug, Serialize)]
struct Effects {
    contract: Effect,
    branch: Effect,
    worktree: Effect,
    binding: Effect,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Effect {
    Created,
    Reused,
}

#[derive(Debug, Serialize)]
struct NextAction {
    cwd: PathBuf,
    argv: [&'static str; 4],
}

struct BootstrapLock {
    _file: File,
}

pub(crate) fn run(repository: &Path, source_path: &Path) -> Result<(), String> {
    let bytes =
        match bounded_file::read_regular(source_path, MAX_CONTRACT_BYTES, "Slice contract input") {
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
    let bytes =
        bounded_file::read_regular(source_path, MAX_CONTRACT_BYTES, "Slice contract input")?;
    prepare_bytes(repository, &bytes)
}

fn prepare_bytes(repository: &Path, bytes: &[u8]) -> Result<ResultRecord, String> {
    let contract: slice_contract::SliceContract = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid Slice contract input: {error}"))?;
    validate_slice_name(&contract.slice)?;
    validate_base_ref_shape(&contract.base_ref)?;

    let repository = slice_worktree::repository_root(repository)?;
    let _lock = acquire_bootstrap_lock(&repository)?;
    let registered = slice_worktree::worktrees(&repository)?;
    let integration = unique_integration_worktree(&registered, &contract.base_ref)?;
    slice_worktree::ensure_clean(&integration.path, "integration worktree", "Slice bootstrap")?;
    slice_contract::validate_contract(&integration.path, &contract)?;

    let workspace = slice_worktree::workspace_root(&repository)?;
    let local = workspace.join(".local-exclude");
    let coordination = local.join("coordination");
    let worktrees_directory = local.join("worktrees");
    let contract_directory = coordination.join(&contract.slice);
    let contract_path = contract_directory.join("slice-contract.json");
    let worktree_path = worktrees_directory.join(&contract.slice);
    let branch_ref = branch_ref(&contract)?;
    slice_worktree::validate_branch_ref(&integration.path, &branch_ref)?;

    let existing_contract = read_optional_contract(&contract_path)?;
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
    } else if path_entry_exists(&worktree_path)? {
        return Err(format!(
            "unregistered Slice worktree path already exists at {}",
            worktree_path.display()
        ));
    }

    verify_active_leases(
        &contract,
        &registered,
        &coordination,
        &contract_path,
        &integration.path,
        &branch_ref,
        &worktree_path,
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
        MAX_CONTRACT_BYTES,
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

    validate_prepared_worktree(
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
    validate_prepared_worktree(
        &integration.path,
        &worktree_path,
        &branch_ref,
        &contract.base,
    )?;
    slice_contract::verify_bound_exact(&worktree_path, &contract_path)?;

    Ok(ResultRecord {
        schema: RESULT_SCHEMA,
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
            contract: effect(contract_created),
            branch: effect(existing_commit.is_none()),
            worktree: effect(worktree_created),
            binding: effect(binding.created),
        },
        next_action: NextAction {
            cwd: worktree_path,
            argv: ["cargo", "xtask", "check", "slice-scope"],
        },
    })
}

fn validate_base_ref_shape(base_ref: &str) -> Result<(), String> {
    if base_ref == "refs/heads/develop" || base_ref.starts_with("refs/heads/wave/") {
        Ok(())
    } else {
        Err("Slice base_ref must identify develop or a Wave integration branch".to_owned())
    }
}

fn validate_slice_name(slice: &str) -> Result<(), String> {
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

fn unique_integration_worktree<'a>(
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

fn branch_ref(contract: &slice_contract::SliceContract) -> Result<String, String> {
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

fn verify_active_leases(
    contract: &slice_contract::SliceContract,
    registered: &[slice_worktree::Worktree],
    coordination: &Path,
    target_contract_path: &Path,
    integration_path: &Path,
    target_branch: &str,
    target_path: &Path,
) -> Result<(), String> {
    let mut checked_contracts =
        verify_coordination_leases(contract, registered, coordination, target_contract_path)?;
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
        let active_path = std::fs::canonicalize(&active.contract_path).map_err(|error| {
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

fn verify_coordination_leases(
    contract: &slice_contract::SliceContract,
    registered: &[slice_worktree::Worktree],
    coordination: &Path,
    target_contract_path: &Path,
) -> Result<BTreeSet<PathBuf>, String> {
    match std::fs::symlink_metadata(coordination) {
        Ok(metadata) if metadata.file_type().is_dir() => {},
        Ok(_) => {
            return Err(format!(
                "coordination path {} must be a directory without symlinks",
                coordination.display()
            ));
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeSet::new());
        },
        Err(error) => {
            return Err(format!(
                "cannot inspect coordination path {}: {error}",
                coordination.display()
            ));
        },
    }
    let entries = match std::fs::read_dir(coordination) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => {
            return Err(format!(
                "cannot inspect coordination directory {}: {error}",
                coordination.display()
            ));
        },
    };
    let mut checked = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "cannot inspect coordination entry in {}: {error}",
                coordination.display()
            )
        })?;
        let entry_type = entry.file_type().map_err(|error| {
            format!(
                "cannot inspect coordination entry {}: {error}",
                entry.path().display()
            )
        })?;
        if entry_type.is_symlink() {
            return Err(format!(
                "coordination entry {} must not be a symlink",
                entry.path().display()
            ));
        }
        if !entry_type.is_dir() {
            continue;
        }
        let path = entry.path().join("slice-contract.json");
        if path == target_contract_path {
            continue;
        }
        let Some(bytes) = read_optional_contract(&path)? else {
            continue;
        };
        let active: slice_contract::SliceContract =
            serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid active Slice contract {}: {error}", path.display())
            })?;
        validate_slice_name(&active.slice)?;
        validate_base_ref_shape(&active.base_ref)?;
        let active_integration = unique_integration_worktree(registered, &active.base_ref)
            .map_err(|error| {
                format!("cannot verify active contract {}: {error}", path.display())
            })?;
        slice_contract::validate_contract(&active_integration.path, &active).map_err(|error| {
            format!("invalid active Slice contract {}: {error}", path.display())
        })?;
        let canonical = std::fs::canonicalize(&path).map_err(|error| {
            format!("cannot resolve active contract {}: {error}", path.display())
        })?;
        checked.insert(canonical);
        slice_contract::ensure_lease_compatible(contract, &active)
            .map_err(|error| format!("{error}\nactive contract: {}", path.display()))?;
    }
    Ok(checked)
}

fn acquire_bootstrap_lock(repository: &Path) -> Result<BootstrapLock, String> {
    let common = git::output_in(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        false,
    )?;
    let common = PathBuf::from(common.trim());
    let lock_path = common.join("yo-slice-create.lock");
    let directory = open(
        &common,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        format!(
            "cannot open common Git directory {}: {error}",
            common.display()
        )
    })?;
    let fd = openat(
        &directory,
        "yo-slice-create.lock",
        OFlags::WRONLY | OFlags::CREATE | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| {
        format!(
            "cannot open Slice bootstrap lock {}: {error}",
            lock_path.display()
        )
    })?;
    let stat =
        fstat(&fd).map_err(|error| format!("cannot inspect Slice bootstrap lock: {error}"))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile || stat.st_nlink != 1 {
        return Err(format!(
            "Slice bootstrap lock {} must be a singly linked regular file",
            lock_path.display()
        ));
    }
    let file = File::from(fd);
    flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
        format!(
            "another cooperating Slice bootstrap is active at {}: {error}",
            lock_path.display()
        )
    })?;
    Ok(BootstrapLock { _file: file })
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
        .ok_or_else(|| "prepared Slice worktree is no longer registered".to_owned())?;
    slice_worktree::validate_coordinates(worktree, expected_path, expected_branch, expected_head)?;
    slice_worktree::ensure_clean(&worktree.path, "Slice worktree", "Slice bootstrap")
}

fn read_optional_contract(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            bounded_file::read_regular(path, MAX_CONTRACT_BYTES, "Slice coordination contract")
                .map(Some)
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn path_entry_exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn effect(created: bool) -> Effect {
    if created {
        Effect::Created
    } else {
        Effect::Reused
    }
}

fn encode_failure(
    repository: &Path,
    bytes: Option<&[u8]>,
    error: String,
) -> Result<String, String> {
    let mut failure = serde_json::json!({
        "schema": RESULT_SCHEMA,
        "ok": false,
        "error": error,
        "effects": {
            "contract": {"state": "unknown"},
            "branch": {"state": "unknown"},
            "worktree": {"state": "unknown"},
            "binding": {"state": "unknown"}
        }
    });
    let Some(bytes) = bytes else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    let Ok(contract) = serde_json::from_slice::<slice_contract::SliceContract>(bytes) else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    failure["slice"] = serde_json::json!(contract.slice);
    failure["base"] = serde_json::json!(contract.base);
    failure["base_ref"] = serde_json::json!(contract.base_ref);
    let Ok(root) = slice_worktree::repository_root(repository) else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    let Ok(workspace) = slice_worktree::workspace_root(&root) else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    let contract_path = workspace
        .join(".local-exclude/coordination")
        .join(&contract.slice)
        .join("slice-contract.json");
    let worktree_path = workspace
        .join(".local-exclude/worktrees")
        .join(&contract.slice);
    let Ok(branch_ref) = branch_ref(&contract) else {
        return serde_json::to_string_pretty(&failure)
            .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"));
    };
    failure["branch_ref"] = serde_json::json!(branch_ref);
    failure["contract_path"] = serde_json::json!(contract_path);
    failure["worktree_path"] = serde_json::json!(worktree_path);
    let prepared_contract = match read_optional_contract(&contract_path) {
        Ok(Some(actual)) if actual == bytes => {
            failure["effects"]["contract"] = serde_json::json!({"state": "prepared"});
            true
        },
        Ok(Some(_)) => {
            failure["effects"]["contract"] = serde_json::json!({"state": "conflicting"});
            false
        },
        Ok(None) => {
            failure["effects"]["contract"] = serde_json::json!({"state": "absent"});
            false
        },
        Err(detail) => {
            failure["effects"]["contract"] =
                serde_json::json!({"state": "conflicting", "detail": detail});
            false
        },
    };
    let branch_prepared = match slice_worktree::existing_ref(&root, &branch_ref) {
        Ok(Some(slice_worktree::ExistingRef::Direct(actual)))
            if prepared_contract && actual == contract.base =>
        {
            failure["effects"]["branch"] = serde_json::json!({"state": "prepared"});
            true
        },
        Ok(Some(slice_worktree::ExistingRef::Direct(actual))) => {
            failure["effects"]["branch"] = serde_json::json!({"state": "conflicting", "detail": format!("branch points to {actual}")});
            false
        },
        Ok(Some(slice_worktree::ExistingRef::Symbolic(target))) => {
            failure["effects"]["branch"] = serde_json::json!({
                "state": "conflicting",
                "detail": format!("branch is symbolic to {target}")
            });
            false
        },
        Ok(None) => {
            failure["effects"]["branch"] = serde_json::json!({"state": "absent"});
            false
        },
        Err(detail) => {
            failure["effects"]["branch"] =
                serde_json::json!({"state": "unknown", "detail": detail});
            false
        },
    };
    let registered = slice_worktree::worktrees(&root).unwrap_or_default();
    let prepared_worktree = registered.iter().any(|worktree| {
        prepared_contract
            && branch_prepared
            && worktree.path == worktree_path
            && worktree.branch.as_deref() == Some(branch_ref.as_str())
            && worktree.head == contract.base
    });
    if prepared_worktree {
        failure["effects"]["worktree"] = serde_json::json!({"state": "prepared"});
        match slice_contract::binding_path_for(&worktree_path) {
            Ok(binding_path) => match std::fs::symlink_metadata(&binding_path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    failure["binding_path"] = serde_json::json!(binding_path);
                    failure["effects"]["binding"] = serde_json::json!({"state": "absent"});
                },
                Ok(_) => match slice_contract::verify_bound_exact(&worktree_path, &contract_path) {
                    Ok(binding_path) => {
                        failure["binding_path"] = serde_json::json!(binding_path);
                        failure["effects"]["binding"] = serde_json::json!({"state": "prepared"});
                    },
                    Err(detail) => {
                        failure["effects"]["binding"] =
                            serde_json::json!({"state": "conflicting", "detail": detail});
                    },
                },
                Err(error) => {
                    failure["effects"]["binding"] = serde_json::json!({
                        "state": "unknown",
                        "detail": format!("cannot inspect {}: {error}", binding_path.display())
                    });
                },
            },
            Err(detail) => {
                failure["effects"]["binding"] =
                    serde_json::json!({"state": "unknown", "detail": detail});
            },
        }
    } else if registered.iter().any(|worktree| {
        worktree.path == worktree_path || worktree.branch.as_deref() == Some(branch_ref.as_str())
    }) || path_entry_exists(&worktree_path).unwrap_or(false)
    {
        failure["effects"]["worktree"] = serde_json::json!({"state": "conflicting"});
    } else {
        failure["effects"]["worktree"] = serde_json::json!({"state": "absent"});
    }
    serde_json::to_string_pretty(&failure)
        .map_err(|error| format!("cannot encode Slice bootstrap failure: {error}"))
}
