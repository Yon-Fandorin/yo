use std::path::{Path, PathBuf};

use super::model::Request;
use crate::{bounded_file, git, review_protocol, slice_contract};

pub(super) const REQUEST_LIMIT: usize = 64 * 1024;
pub(super) const EVIDENCE_LIMIT: usize = 64 * 1024;
pub(super) const MAX_PATHS: usize = 256;
pub(super) const MAX_PATH_BYTES: usize = 32 * 1024;

pub(super) fn require_clean(repository: &Path) -> Result<(), String> {
    let status = git::trusted_output_bytes_in(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err("Slice gate requires a clean candidate worktree".to_owned())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn final_revalidate(
    repository: &Path,
    request_path: &Path,
    request_bytes: &[u8],
    bound: &slice_contract::BoundSlice,
    candidate: &str,
    diff_hash: &str,
    changed: &[String],
    request: &Request,
) -> Result<(), String> {
    let current_request =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice gate request")?;
    if current_request != request_bytes {
        return Err("Slice gate request changed during evaluation".to_owned());
    }
    if slice_contract::trusted_bound_slice(repository)? != *bound {
        return Err("bound Slice identity changed during gate evaluation".to_owned());
    }
    slice_contract::trusted_check_bound_scope(repository)?;
    require_clean(repository)?;
    if trusted_line(repository, &["rev-parse", "--verify", "HEAD^{commit}"])? != candidate {
        return Err("candidate HEAD changed during gate evaluation".to_owned());
    }
    let current_changed = changed_paths(repository, &bound.base, candidate)?;
    if current_changed != changed {
        return Err("candidate changed paths changed during gate evaluation".to_owned());
    }
    let current_diff = git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            &bound.base,
            candidate,
            "--",
        ],
    )?;
    if review_protocol::digest(&current_diff) != diff_hash {
        return Err("canonical candidate diff changed during gate evaluation".to_owned());
    }
    for entry in &request.validation_evidence {
        let _ = captured(
            repository,
            &entry.result_path,
            &entry.result_hash,
            "validation result",
        )?;
    }
    for entry in &request.review_evidence {
        let _ = captured(
            repository,
            &entry.result_path,
            &entry.result_hash,
            "review result",
        )?;
    }
    Ok(())
}
pub(super) fn changed_paths(
    repository: &Path,
    base: &str,
    candidate: &str,
) -> Result<Vec<String>, String> {
    let bytes = git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            base,
            candidate,
            "--",
        ],
    )?;
    let mut paths = bytes
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| {
            String::from_utf8(value.to_vec())
                .map_err(|_| "Slice gate does not support non-UTF-8 changed paths".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    paths.dedup();
    if paths.len() > MAX_PATHS || paths.iter().map(String::len).sum::<usize>() > MAX_PATH_BYTES {
        return Err(format!(
            "changed paths exceed the bounded result limit ({MAX_PATHS} paths or {MAX_PATH_BYTES} bytes)"
        ));
    }
    Ok(paths)
}

pub(super) fn captured(
    repository: &Path,
    value: &str,
    expected: &str,
    label: &str,
) -> Result<Vec<u8>, String> {
    canonical_sha256(expected, &format!("{label} hash"))?;
    compact(value, 4096, &format!("{label} path"))?;
    let path = resolve_path(repository, value);
    let bytes = bounded_file::read_regular(&path, EVIDENCE_LIMIT, label)?;
    let actual = review_protocol::digest(&bytes);
    if actual != expected {
        return Err(format!(
            "{label} {} hash changed: expected {expected}, found {actual}",
            path.display()
        ));
    }
    Ok(bytes)
}

fn resolve_path(repository: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository.join(path)
    }
}

pub(super) fn exact_candidate(value: &str, candidate: &str, label: &str) -> Result<(), String> {
    review_protocol::require_commit(value, &format!("{label} candidate_commit"))?;
    if value == candidate {
        Ok(())
    } else {
        Err(format!(
            "{label} is stale: candidate {value} does not match {candidate}"
        ))
    }
}

fn canonical_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    }) {
        Ok(())
    } else {
        Err(format!("{label} must be canonical SHA-256"))
    }
}

pub(super) fn compact(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err(format!(
            "{label} must contain 1..={max} bytes without control characters"
        ))
    } else {
        Ok(())
    }
}

pub(super) fn compact_id(value: &str, max: usize, label: &str) -> Result<(), String> {
    compact(value, max, label)?;
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._/+:-".contains(character))
    {
        Ok(())
    } else {
        Err(format!("{label} contains unsupported characters"))
    }
}

pub(super) fn trusted_line(repository: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = git::trusted_output_in(repository, arguments)?;
    let value = output.trim();
    if value.is_empty() || output.lines().count() != 1 {
        Err(format!(
            "trusted Git {} returned an invalid line",
            arguments.join(" ")
        ))
    } else {
        Ok(value.to_owned())
    }
}
