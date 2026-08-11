use std::path::{Path, PathBuf};

use crate::{git, review_protocol::require_commit};

pub(super) fn capture_delta(
    repository: &Path,
    prior: &str,
    replacement: &str,
) -> Result<Vec<u8>, String> {
    if !git::trusted_succeeds_in(
        repository,
        &["merge-base", "--is-ancestor", prior, replacement],
    )? {
        return Err("prior candidate is not an ancestor of the replacement candidate".to_owned());
    }
    git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            prior,
            replacement,
            "--",
        ],
    )
}

pub(super) fn require_expected_branch(
    repository: &Path,
    base_ref: &str,
    slice: &str,
) -> Result<(), String> {
    let expected = if base_ref == "refs/heads/develop" {
        format!("refs/heads/slice/direct/{slice}")
    } else {
        let wave = base_ref
            .strip_prefix("refs/heads/wave/")
            .filter(|wave| !wave.is_empty() && !wave.contains('/'))
            .ok_or_else(|| format!("unsupported Slice integration ref `{base_ref}`"))?;
        format!("refs/heads/slice/{wave}/{slice}")
    };
    let actual = git::trusted_output_in(repository, &["symbolic-ref", "--quiet", "HEAD"])?;
    if actual.trim() == expected {
        Ok(())
    } else {
        Err(format!(
            "trusted Git branch does not match bound Slice; expected {expected}"
        ))
    }
}

pub(super) fn trusted_resolve_commit(repository: &Path, reference: &str) -> Result<String, String> {
    let value = git::trusted_output_in(
        repository,
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )?;
    let value = value.trim().to_owned();
    require_commit(&value, "resolved commit")?;
    Ok(value)
}

pub(super) fn trusted_repository_root(directory: &Path) -> Result<PathBuf, String> {
    let root = git::trusted_output_in(directory, &["rev-parse", "--show-toplevel"])?;
    let root = root.trim();
    if root.is_empty() {
        return Err("trusted Git returned an empty repository root".to_owned());
    }
    Ok(PathBuf::from(root))
}

pub(super) fn trusted_ensure_clean(repository: &Path, operation: &str) -> Result<(), String> {
    let status = git::trusted_output_bytes_in(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "candidate worktree must be clean before {operation}"
        ))
    }
}
