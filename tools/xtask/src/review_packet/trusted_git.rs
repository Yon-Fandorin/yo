use std::path::{Path, PathBuf};

use crate::{git, review_protocol::require_commit};

pub(super) fn trusted_resolve_commit(repository: &Path, reference: &str) -> Result<String, String> {
    let value = trusted_git_text(
        repository,
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )?;
    let value = value.trim().to_owned();
    require_commit(&value, "resolved commit")?;
    Ok(value)
}

pub(super) fn trusted_git_succeeds(repository: &Path, arguments: &[&str]) -> Result<bool, String> {
    git::trusted_succeeds_in(repository, arguments)
}

pub(super) fn trusted_git_text(repository: &Path, arguments: &[&str]) -> Result<String, String> {
    git::trusted_output_in(repository, arguments)
}

pub(super) fn trusted_git_bytes(repository: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    git::trusted_output_bytes_in(repository, arguments)
}

pub(super) fn trusted_repository_root(directory: &Path) -> Result<PathBuf, String> {
    let root = trusted_git_text(directory, &["rev-parse", "--show-toplevel"])?;
    let root = root.trim();
    if root.is_empty() {
        return Err("trusted Git returned an empty repository root".to_owned());
    }
    Ok(PathBuf::from(root))
}

pub(super) fn trusted_ensure_clean(
    repository: &Path,
    label: &str,
    operation: &str,
) -> Result<(), String> {
    let status = trusted_git_bytes(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(format!("{label} must be clean before {operation}"))
    }
}

pub(super) fn expected_slice_ref(base_ref: &str, slice: &str) -> Result<String, String> {
    if base_ref == "refs/heads/develop" {
        return Ok(format!("refs/heads/slice/direct/{slice}"));
    }
    let wave = base_ref
        .strip_prefix("refs/heads/wave/")
        .filter(|wave| !wave.is_empty() && !wave.contains('/'))
        .ok_or_else(|| format!("unsupported Slice integration ref `{base_ref}`"))?;
    Ok(format!("refs/heads/slice/{wave}/{slice}"))
}
