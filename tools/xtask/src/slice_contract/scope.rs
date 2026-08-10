use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use super::{
    binding, json,
    model::{self, SliceContract},
    repository_root, repository_root_with,
};
use crate::git;

pub(crate) fn check_scope(repository: &Path, contract_path: &Path) -> Result<(), String> {
    let index_file = selected_index(repository);
    check_scope_with_index(repository, contract_path, index_file.as_deref())
}

pub(crate) fn check_bound_scope(repository: &Path) -> Result<(), String> {
    let index_file = selected_index(repository);
    check_bound_scope_with_index(repository, index_file.as_deref())
}

pub(crate) fn trusted_check_bound_scope(repository: &Path) -> Result<(), String> {
    let repository = repository_root_with(repository, true)?;
    let contract_path = binding::bound_contract_path_with(&repository, true)?;
    let contract = model::load(&contract_path)?;
    model::validate_with(&repository, &contract, true)?;
    model::validate_slice_branch_with(&repository, &contract, true)?;
    let bytes = git::trusted_output_bytes_in(
        &repository,
        &[
            "diff",
            "--name-status",
            "-z",
            "--no-renames",
            &contract.base,
            "HEAD",
            "--",
        ],
    )?;
    let changed = diff_paths(bytes)?;
    require_allowed_paths(&contract, &changed)
}

fn selected_index(repository: &Path) -> Option<PathBuf> {
    std::env::var_os("GIT_INDEX_FILE").map(|value| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            repository.join(path)
        }
    })
}

pub(super) fn check_bound_scope_with_index(
    repository: &Path,
    index_file: Option<&Path>,
) -> Result<(), String> {
    let contract_path = binding::bound_contract_path(repository)?;
    let contract = model::load(&contract_path)?;
    let repository = repository_root(repository)?;
    model::validate_slice_branch(&repository, &contract)?;
    check_scope_with_index(&repository, &contract_path, index_file)
}

pub(super) fn check_scope_with_index(
    repository: &Path,
    contract_path: &Path,
    index_file: Option<&Path>,
) -> Result<(), String> {
    let contract = model::load(contract_path)?;
    let repository = repository_root(repository)?;
    model::validate(&repository, &contract)?;
    reject_hidden_index_paths(&repository, index_file.map(Path::as_os_str))?;

    let changed = changed_paths(&repository, &contract.base, index_file.map(Path::as_os_str))?;
    require_allowed_paths(&contract, &changed)?;

    println!(
        "{{\"schema\":\"yo.slice-scope-check/v1\",\"ok\":true,\"slice\":{},\"contract_path\":{},\"base\":{},\"changed_paths\":{}}}",
        json(&contract.slice)?,
        json(&contract_path)?,
        json(&contract.base)?,
        json(&changed)?
    );
    Ok(())
}

fn require_allowed_paths(contract: &SliceContract, changed: &[String]) -> Result<(), String> {
    let rules = model::parse_rules(&contract.allowed_write_set)?;
    let outside = changed
        .iter()
        .filter(|path| !rules.iter().any(|rule| rule.matches(path)))
        .cloned()
        .collect::<Vec<_>>();
    if !outside.is_empty() {
        return Err(format!(
            "Slice `{}` changed paths outside its allowed write-set:\n{}",
            contract.slice,
            outside
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    Ok(())
}

fn changed_paths(
    repository: &Path,
    base: &str,
    index_file: Option<&OsStr>,
) -> Result<Vec<String>, String> {
    let mut paths = diff_paths(git::output_bytes_in_with_index(
        repository,
        &[
            "diff",
            "--name-status",
            "-z",
            "--no-renames",
            base,
            "HEAD",
            "--",
        ],
        index_file,
    )?)?;
    paths.extend(diff_paths(git::output_bytes_in_with_index(
        repository,
        &[
            "diff",
            "--cached",
            "--name-status",
            "-z",
            "--no-renames",
            "HEAD",
            "--",
        ],
        index_file,
    )?)?);
    paths.extend(diff_paths(git::output_bytes_in_with_index(
        repository,
        &["diff", "--name-status", "-z", "--no-renames", "--"],
        index_file,
    )?)?);
    paths.extend(nul_paths(git::output_bytes_in_with_index(
        repository,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        index_file,
    )?)?);
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn reject_hidden_index_paths(repository: &Path, index_file: Option<&OsStr>) -> Result<(), String> {
    let entries =
        git::output_bytes_in_with_index(repository, &["ls-files", "-v", "-z"], index_file)?;
    let hidden = entries
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let (&tag, path) = entry.split_first()?;
            let path = path.strip_prefix(b" ")?;
            (tag == b'S' || tag.is_ascii_lowercase()).then_some(path)
        })
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map_err(|_| "Git returned a non-UTF-8 hidden index path".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if hidden.is_empty() {
        return Ok(());
    }

    Err(format!(
        "Slice scope cannot observe paths marked assume-unchanged or skip-worktree:\n{}",
        hidden
            .iter()
            .map(|path| format!("- {path}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn diff_paths(bytes: Vec<u8>) -> Result<Vec<String>, String> {
    let mut fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut paths = Vec::new();
    while let Some(status) = fields.next() {
        if status.len() != 1 || !status[0].is_ascii_uppercase() {
            return Err("Git returned an invalid name-status record".to_owned());
        }
        let path = fields
            .next()
            .ok_or_else(|| "Git returned a name-status record without a path".to_owned())?;
        paths.push(
            String::from_utf8(path.to_vec())
                .map_err(|_| "Git returned a non-UTF-8 changed path".to_owned())?,
        );
    }
    Ok(paths)
}

fn nul_paths(bytes: Vec<u8>) -> Result<Vec<String>, String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map_err(|_| "Git returned a non-UTF-8 changed path".to_owned())
        })
        .collect()
}
