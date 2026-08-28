use std::path::{Path, PathBuf};

use crate::git;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Worktree {
    pub(crate) path: PathBuf,
    pub(crate) head: String,
    pub(crate) branch: Option<String>,
}

pub(crate) enum ExistingRef {
    Direct(String),
    Symbolic(String),
}

pub(crate) fn worktrees(repository: &Path) -> Result<Vec<Worktree>, String> {
    let bytes = git::output_bytes_in(
        repository,
        &["worktree", "list", "--porcelain", "-z"],
        false,
    )?;
    let mut found = Vec::new();
    for record in bytes
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>()
        .split(|field| field.is_empty())
    {
        if record.is_empty() {
            continue;
        }
        let mut path = None;
        let mut head = None;
        let mut branch = None;
        for field in record {
            if let Some(value) = field.strip_prefix(b"worktree ") {
                path = Some(path_from_bytes(value)?);
            } else if let Some(value) = field.strip_prefix(b"HEAD ") {
                head = Some(text_from_bytes(value, "worktree HEAD")?);
            } else if let Some(value) = field.strip_prefix(b"branch ") {
                branch = Some(text_from_bytes(value, "worktree branch")?);
            }
        }
        if let (Some(path), Some(head)) = (path, head) {
            found.push(Worktree { path, head, branch });
        }
    }
    Ok(found)
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec())))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, String> {
    Ok(PathBuf::from(text_from_bytes(bytes, "worktree path")?))
}

fn text_from_bytes(bytes: &[u8], label: &str) -> Result<String, String> {
    String::from_utf8(bytes.to_vec())
        .map_err(|error| format!("Git returned non-UTF-8 {label}: {error}"))
}

pub(crate) fn current_branch_ref(repository: &Path, operation: &str) -> Result<String, String> {
    git::output_in(repository, &["symbolic-ref", "--quiet", "HEAD"], false)
        .map(|value| value.trim().to_owned())
        .map_err(|_| format!("run {operation} from a named integration branch"))
}

pub(crate) fn repository_root(repository: &Path) -> Result<PathBuf, String> {
    let root = git::output_in(repository, &["rev-parse", "--show-toplevel"], false)?;
    let root = root.trim();
    if root.is_empty() {
        return Err("git rev-parse returned an empty repository root".to_owned());
    }
    Ok(PathBuf::from(root))
}

pub(crate) fn workspace_root(repository: &Path) -> Result<PathBuf, String> {
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

pub(crate) fn resolve_commit(repository: &Path, reference: &str) -> Result<String, String> {
    git::output_in(
        repository,
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
        false,
    )
    .map(|value| value.trim().to_owned())
}

pub(crate) fn expect_ref(repository: &Path, reference: &str, expected: &str) -> Result<(), String> {
    let actual = resolve_commit(repository, reference)?;
    if actual != expected {
        return Err(format!(
            "{reference} changed after planning: expected {expected}, found {actual}"
        ));
    }
    Ok(())
}

pub(crate) fn ensure_clean(repository: &Path, label: &str, operation: &str) -> Result<(), String> {
    let status = git::output_bytes_in(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        false,
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(format!("{label} must be clean before {operation}"))
    }
}

pub(crate) fn validate_branch_ref(repository: &Path, branch_ref: &str) -> Result<(), String> {
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

pub(crate) fn existing_ref(
    repository: &Path,
    reference: &str,
) -> Result<Option<ExistingRef>, String> {
    if let Some(target) = symbolic_ref_target(repository, reference)? {
        return Ok(Some(ExistingRef::Symbolic(target)));
    }
    if git::succeeds_in(
        repository,
        &["show-ref", "--verify", "--quiet", reference],
        false,
    )? {
        resolve_commit(repository, reference)
            .map(ExistingRef::Direct)
            .map(Some)
    } else {
        Ok(None)
    }
}

fn symbolic_ref_target(repository: &Path, reference: &str) -> Result<Option<String>, String> {
    let output = git::command_in(repository, false)
        .args(["symbolic-ref", "--quiet", reference])
        .output()
        .map_err(|error| format!("cannot inspect Slice ref `{reference}`: {error}"))?;
    if output.status.success() {
        let target = String::from_utf8(output.stdout)
            .map_err(|error| format!("Git returned a non-UTF-8 symbolic ref target: {error}"))?;
        let target = target.trim();
        if target.is_empty() {
            Err(format!(
                "Git returned an empty symbolic ref target for `{reference}`"
            ))
        } else {
            Ok(Some(target.to_owned()))
        }
    } else if output.status.code() == Some(1) {
        Ok(None)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "cannot inspect Slice ref `{reference}` with Git symbolic-ref: {}{}",
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ))
    }
}

pub(crate) fn validate_coordinates(
    worktree: &Worktree,
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

pub(crate) fn create(
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
