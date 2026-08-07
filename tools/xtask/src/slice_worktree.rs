use std::path::{Path, PathBuf};

use crate::git;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Worktree {
    pub(crate) path: PathBuf,
    pub(crate) head: String,
    pub(crate) branch: Option<String>,
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
