use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};

use rustix::fs::{FileType, FlockOperation, Mode, OFlags, flock, fstat, open, openat};
pub(super) use slice_worktree::Worktree;

use crate::{git, slice_worktree};

const CLOSE_METRICS_CUTOVER_MARKER: &str = "tools/xtask/src/slice_close/metrics-cutover";
const _: &[u8] = include_bytes!("metrics-cutover");

pub(super) struct CleanupLock {
    _file: File,
}

pub(super) fn acquire_cleanup_lock(repository: &Path) -> Result<CleanupLock, String> {
    let common = git::output_in(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        false,
    )?;
    let common = PathBuf::from(common.trim());
    let path = common.join("yo-slice-close.lock");
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
        "yo-slice-close.lock",
        OFlags::WRONLY | OFlags::CREATE | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| format!("cannot open Slice close lock {}: {error}", path.display()))?;
    let stat = fstat(&fd).map_err(|error| {
        format!(
            "cannot inspect Slice close lock {}: {error}",
            path.display()
        )
    })?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile || stat.st_nlink != 1 {
        return Err(format!(
            "Slice close lock {} must be a singly linked regular file",
            path.display()
        ));
    }
    let file = File::from(fd);
    flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
        format!(
            "another cooperating Slice close is active at {}: {error}",
            path.display()
        )
    })?;
    Ok(CleanupLock { _file: file })
}

pub(super) fn find_accepted_commit(
    repository: &Path,
    integration_head: &str,
    slice_base: &str,
    slice_head: &str,
) -> Result<(String, String), String> {
    let slice_patch = required_patch_id(repository, slice_base, slice_head)?;
    let commits = git::output_in(
        repository,
        &[
            "rev-list",
            "--first-parent",
            "--reverse",
            &format!("{slice_base}..{integration_head}"),
        ],
        false,
    )?;
    let mut matches = Vec::new();
    for commit in commits.lines().filter(|line| !line.is_empty()) {
        let Some(parent) = single_parent(repository, commit)? else {
            continue;
        };
        if patch_id(repository, &parent, commit)?.as_deref() == Some(slice_patch.as_str()) {
            matches.push(commit.to_owned());
        }
    }
    match matches.as_slice() {
        [accepted] => Ok((slice_patch, accepted.clone())),
        [] => Err(format!(
            "no accepted commit through {integration_head} has the Slice patch {slice_patch}"
        )),
        _ => Err(format!(
            "multiple accepted commits have the Slice patch {slice_patch}; cleanup is ambiguous"
        )),
    }
}

pub(super) fn matching_patch_id(
    repository: &Path,
    slice_base: &str,
    slice_head: &str,
    accepted_commit: &str,
) -> Result<String, String> {
    let parent = single_parent(repository, accepted_commit)?.ok_or_else(|| {
        format!("accepted integration commit {accepted_commit} must have exactly one parent")
    })?;
    let slice_patch = required_patch_id(repository, slice_base, slice_head)?;
    let accepted_patch = required_patch_id(repository, &parent, accepted_commit)?;
    if slice_patch != accepted_patch {
        return Err(format!(
            "Slice patch {slice_patch} does not match accepted commit patch {accepted_patch}"
        ));
    }
    Ok(slice_patch)
}

pub(super) fn accepted_commit_requires_close_metrics(
    repository: &Path,
    accepted_commit: &str,
) -> Result<bool, String> {
    let marker = git::output_in(
        repository,
        &[
            "ls-tree",
            "--name-only",
            accepted_commit,
            "--",
            CLOSE_METRICS_CUTOVER_MARKER,
        ],
        false,
    )?;
    Ok(marker
        .lines()
        .any(|path| path == CLOSE_METRICS_CUTOVER_MARKER))
}

fn single_parent(repository: &Path, commit: &str) -> Result<Option<String>, String> {
    let parents = git::output_in(
        repository,
        &["rev-list", "--parents", "-n", "1", commit],
        false,
    )?;
    let parents = parents.split_whitespace().collect::<Vec<_>>();
    match parents.as_slice() {
        [_, parent] => Ok(Some((*parent).to_owned())),
        [_, _, ..] => Ok(None),
        _ => Err(format!("cannot read parent of integration commit {commit}")),
    }
}

fn required_patch_id(repository: &Path, from: &str, to: &str) -> Result<String, String> {
    patch_id(repository, from, to)?
        .ok_or_else(|| "git patch-id returned no valid patch identity".to_owned())
}

fn patch_id(repository: &Path, from: &str, to: &str) -> Result<Option<String>, String> {
    let mut diff = git::command_in(repository, false)
        .args(["diff", "--binary", "--full-index", from, to, "--"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start git diff: {error}"))?;
    let diff_stdout = diff
        .stdout
        .take()
        .expect("piped git diff stdout is available");
    let output = git::command_in(repository, false)
        .args(["patch-id", "--verbatim"])
        .stdin(Stdio::from(diff_stdout))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("cannot wait for git patch-id: {error}"))?;
    let diff = diff
        .wait_with_output()
        .map_err(|error| format!("cannot wait for git diff: {error}"))?;
    if !diff.status.success() {
        return Err(format!(
            "git diff failed with {}: {}",
            diff.status,
            String::from_utf8_lossy(&diff.stderr).trim()
        ));
    }
    if !output.status.success() {
        return Err(format!(
            "git patch-id failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|error| format!("git patch-id returned non-UTF-8 output: {error}"))?;
    Ok(output
        .split_whitespace()
        .next()
        .filter(|value| {
            matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .map(str::to_owned))
}

pub(super) fn current_branch_ref(repository: &Path) -> Result<String, String> {
    slice_worktree::current_branch_ref(repository, "Slice close")
}

pub(super) fn repository_root(repository: &Path) -> Result<PathBuf, String> {
    slice_worktree::repository_root(repository)
}

pub(super) fn resolve_commit(repository: &Path, reference: &str) -> Result<String, String> {
    slice_worktree::resolve_commit(repository, reference)
}

pub(super) fn expect_ref(repository: &Path, reference: &str, expected: &str) -> Result<(), String> {
    slice_worktree::expect_ref(repository, reference, expected)
}

pub(super) fn ensure_clean(repository: &Path, label: &str) -> Result<(), String> {
    slice_worktree::ensure_clean(repository, label, "Slice close")
}

pub(super) fn worktrees(repository: &Path) -> Result<Vec<Worktree>, String> {
    slice_worktree::worktrees(repository)
}

pub(super) fn remove_worktree(repository: &Path, path: &Path) -> Result<(), String> {
    run_git(
        repository,
        &["worktree", "remove", "--"],
        Some(path),
        "git worktree remove",
    )
}

pub(super) fn delete_slice_ref_guarded(
    repository: &Path,
    integration_ref: &str,
    integration_head: &str,
    slice_ref: &str,
    slice_head: &str,
) -> Result<(), String> {
    let transaction = format!(
        "start\nverify {integration_ref} {integration_head}\n\
         delete {slice_ref} {slice_head}\nprepare\ncommit\n"
    );
    let mut child = git::command_in(repository, false)
        .args(["update-ref", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start guarded git update-ref: {error}"))?;
    child
        .stdin
        .take()
        .expect("piped update-ref stdin is available")
        .write_all(transaction.as_bytes())
        .map_err(|error| format!("cannot write guarded git update-ref transaction: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot wait for guarded git update-ref: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "guarded git update-ref failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn run_git(
    repository: &Path,
    arguments: &[&str],
    path: Option<&Path>,
    label: &str,
) -> Result<(), String> {
    let mut command = git::command_in(repository, false);
    command.args(arguments);
    if let Some(path) = path {
        command.arg(path);
    }
    let status = command
        .status()
        .map_err(|error| format!("cannot run {label}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed with {status}"))
    }
}
