use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{
    git_output, json,
    model::{self, BoundSlice, EnsuredBinding},
    repository_root, repository_root_with,
};
use crate::bounded_file;

pub(crate) fn bound_slice(repository: &Path) -> Result<BoundSlice, String> {
    bound_slice_with(repository, false)
}

pub(crate) fn trusted_bound_slice(repository: &Path) -> Result<BoundSlice, String> {
    bound_slice_with(repository, true)
}

fn bound_slice_with(repository: &Path, trusted: bool) -> Result<BoundSlice, String> {
    let repository = repository_root_with(repository, trusted)?;
    let binding_path = binding_path_with(&repository, trusted)?;
    let contract_path = bound_contract_path_with(&repository, trusted)?;
    let (contract, bytes) = model::read_contract(&contract_path)?;
    model::validate_with(&repository, &contract, trusted)?;
    model::validate_slice_branch_with(&repository, &contract, trusted)?;
    Ok(BoundSlice {
        slice: contract.slice,
        base: contract.base,
        base_ref: contract.base_ref,
        binding_path,
        contract_path,
        contract_id: format!(
            "sha256:{}",
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
    })
}

pub(crate) fn bind(repository: &Path, contract_path: &Path) -> Result<(), String> {
    let contract_path = std::fs::canonicalize(contract_path).map_err(|error| {
        format!(
            "cannot resolve Slice contract {}: {error}",
            contract_path.display()
        )
    })?;
    let contract = model::load(&contract_path)?;
    let repository = repository_root(repository)?;
    model::validate(&repository, &contract)?;
    model::validate_slice_branch(&repository, &contract)?;

    let binding = binding_path(&repository)?;
    std::fs::write(&binding, format!("{}\n", contract_path.display())).map_err(|error| {
        format!(
            "cannot bind Slice contract at {}: {error}",
            binding.display()
        )
    })?;

    println!(
        "{{\"schema\":\"yo.slice-contract-binding/v1\",\"ok\":true,\"slice\":{},\"contract_path\":{}}}",
        json(&contract.slice)?,
        json(&contract_path)?
    );
    Ok(())
}

pub(crate) fn verify_bound_exact(
    repository: &Path,
    contract_path: &Path,
) -> Result<PathBuf, String> {
    let contract_path = std::fs::canonicalize(contract_path).map_err(|error| {
        format!(
            "cannot resolve Slice contract {}: {error}",
            contract_path.display()
        )
    })?;
    let repository = repository_root(repository)?;
    let binding = binding_path(&repository)?;
    let expected = format!("{}\n", contract_path.display());
    let actual = bounded_file::read_regular(&binding, 64 * 1024, "Slice contract binding")?;
    if actual == expected.as_bytes() {
        Ok(binding)
    } else {
        Err(format!(
            "Slice contract binding {} already contains different bytes",
            binding.display()
        ))
    }
}

pub(crate) fn binding_path_for(repository: &Path) -> Result<PathBuf, String> {
    let repository = repository_root(repository)?;
    binding_path(&repository)
}

pub(crate) fn ensure_bound(
    repository: &Path,
    contract_path: &Path,
) -> Result<EnsuredBinding, String> {
    let contract_path = std::fs::canonicalize(contract_path).map_err(|error| {
        format!(
            "cannot resolve Slice contract {}: {error}",
            contract_path.display()
        )
    })?;
    let contract = model::load(&contract_path)?;
    let repository = repository_root(repository)?;
    model::validate(&repository, &contract)?;
    model::validate_slice_branch(&repository, &contract)?;

    let binding = binding_path(&repository)?;
    let expected = format!("{}\n", contract_path.display());
    let created = bounded_file::publish_new_or_exact(
        &binding,
        expected.as_bytes(),
        64 * 1024,
        "Slice contract binding",
    )?;
    Ok(EnsuredBinding {
        slice: contract.slice,
        contract_path,
        binding_path: binding,
        created,
    })
}

pub(super) fn bound_contract_path(repository: &Path) -> Result<PathBuf, String> {
    let repository = repository_root(repository)?;
    bound_contract_path_with(&repository, false)
}

pub(super) fn bound_contract_path_with(
    repository: &Path,
    trusted: bool,
) -> Result<PathBuf, String> {
    let binding = binding_path_with(repository, trusted)?;
    let value = std::fs::read_to_string(&binding).map_err(|error| {
        format!(
            "this worktree has no readable Slice contract binding at {}: {error}\n\
             ask the planner to run `cargo xtask slice-contract bind <slice-contract.json>`",
            binding.display()
        )
    })?;
    let path = value.trim();
    if path.is_empty() || value.lines().count() != 1 {
        return Err(format!(
            "Slice contract binding {} must contain exactly one non-empty path",
            binding.display()
        ));
    }
    Ok(PathBuf::from(path))
}

fn binding_path(repository: &Path) -> Result<PathBuf, String> {
    binding_path_with(repository, false)
}

fn binding_path_with(repository: &Path, trusted: bool) -> Result<PathBuf, String> {
    let arguments = [
        "rev-parse",
        "--path-format=absolute",
        "--git-path",
        model::BINDING_FILE,
    ];
    let output = git_output(repository, &arguments, trusted)?;
    let path = output.trim();
    if path.is_empty() {
        return Err("git rev-parse --git-path returned an empty binding path".to_owned());
    }
    Ok(PathBuf::from(path))
}
