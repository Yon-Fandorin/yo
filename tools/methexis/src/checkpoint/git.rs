//! Exact Git-object snapshot resolution without checkout mutation.

use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{MAX_RECORD_BYTES, OperationFailure};

static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const SYSTEM_GIT: &str = "/usr/bin/git";

pub(super) struct TrustedSnapshot {
    pub(super) commit: String,
    pub(super) root: PathBuf,
}

impl Drop for TrustedSnapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn resolve(
    repository_root: &Path,
    trusted_ref: &str,
    operation: &'static str,
) -> Result<TrustedSnapshot, OperationFailure> {
    let commit = git_output(
        repository_root,
        &[
            "rev-parse",
            "--verify",
            &format!("{trusted_ref}^{{commit}}"),
        ],
        operation,
        None,
        "trusted_ref_unavailable",
    )?;
    let commit = String::from_utf8(commit)
        .map_err(|error| {
            OperationFailure::new(
                operation,
                None,
                "invalid_git_output",
                error.to_string(),
                Vec::new(),
                "repair the repository",
            )
        })?
        .trim()
        .to_owned();
    if !valid_commit(&commit) {
        return Err(OperationFailure::new(
            operation,
            None,
            "invalid_trusted_commit",
            "trusted ref did not resolve to a hexadecimal commit ID",
            Vec::new(),
            "repair the trusted ref",
        ));
    }

    let listing = git_output(
        repository_root,
        &[
            "ls-tree",
            "-rz",
            "-l",
            "--full-tree",
            &commit,
            "--",
            "methexis",
        ],
        operation,
        Some(commit.clone()),
        "trusted_snapshot_unreadable",
    )?;
    let root = create_snapshot_root(operation, &commit)?;
    if let Err(error) = materialize(repository_root, operation, &commit, &listing, &root) {
        let _ = fs::remove_dir_all(&root);
        return Err(error);
    }
    Ok(TrustedSnapshot { commit, root })
}

pub(super) fn resolve_exact(
    repository_root: &Path,
    commit: &str,
    operation: &'static str,
) -> Result<TrustedSnapshot, OperationFailure> {
    if !valid_commit(commit) {
        return Err(OperationFailure::new(
            operation,
            None,
            "invalid_trusted_commit",
            "Checkpoint trusted commit is not a lowercase Git object ID",
            Vec::new(),
            "recreate the Checkpoint from trusted integration",
        ));
    }
    let snapshot = resolve(repository_root, commit, operation)?;
    if snapshot.commit != commit {
        return Err(OperationFailure::new(
            operation,
            Some(snapshot.commit.clone()),
            "checkpoint_trust_mismatch",
            "Checkpoint trusted commit did not resolve to the exact recorded object",
            Vec::new(),
            "recreate the Checkpoint from trusted integration",
        ));
    }
    Ok(snapshot)
}

pub(super) fn require_ancestor(
    repository_root: &Path,
    ancestor: &str,
    descendant: &str,
    operation: &'static str,
) -> Result<(), OperationFailure> {
    git_output(
        repository_root,
        &["merge-base", "--is-ancestor", ancestor, descendant],
        operation,
        Some(descendant.to_owned()),
        "checkpoint_trust_mismatch",
    )
    .map(|_| ())
}

fn materialize(
    repository_root: &Path,
    operation: &'static str,
    commit: &str,
    listing: &[u8],
    root: &Path,
) -> Result<(), OperationFailure> {
    let mut count = 0usize;
    for entry in listing
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        count += 1;
        let separator = entry
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| {
                failure(
                    operation,
                    commit,
                    "invalid_git_tree",
                    "Git tree entry has no path",
                )
            })?;
        let (header, raw_path) = (&entry[..separator], &entry[separator + 1..]);
        let header = std::str::from_utf8(header)
            .map_err(|error| failure(operation, commit, "invalid_git_tree", &error.to_string()))?;
        let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 4 || fields[1] != "blob" || fields[0] != "100644" {
            return Err(failure(
                operation,
                commit,
                "unsupported_git_entry",
                "authority tree entries must be regular non-executable blobs",
            ));
        }
        let size = fields[3]
            .parse::<usize>()
            .map_err(|error| failure(operation, commit, "invalid_git_tree", &error.to_string()))?;
        if size > MAX_RECORD_BYTES {
            return Err(failure(
                operation,
                commit,
                "trusted_record_too_large",
                "trusted record exceeds the Pilot size limit",
            ));
        }
        let path = std::str::from_utf8(raw_path)
            .map_err(|error| failure(operation, commit, "invalid_git_path", &error.to_string()))?;
        let relative = Path::new(path);
        if !safe_relative(relative) {
            return Err(failure(
                operation,
                commit,
                "invalid_git_path",
                "trusted record path is not a safe repository-relative path",
            ));
        }
        let bytes = git_output(
            repository_root,
            &["cat-file", "blob", fields[2]],
            operation,
            Some(commit.to_owned()),
            "trusted_snapshot_unreadable",
        )?;
        if bytes.len() != size {
            return Err(failure(
                operation,
                commit,
                "git_object_size_mismatch",
                "Git blob size differs from the pinned tree entry",
            ));
        }
        let target = root.join(relative);
        let parent = target.parent().ok_or_else(|| {
            failure(
                operation,
                commit,
                "invalid_git_path",
                "trusted record has no parent",
            )
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            failure(
                operation,
                commit,
                "snapshot_materialization_failed",
                &error.to_string(),
            )
        })?;
        fs::write(target, bytes).map_err(|error| {
            failure(
                operation,
                commit,
                "snapshot_materialization_failed",
                &error.to_string(),
            )
        })?;
    }
    if count == 0 {
        return Err(failure(
            operation,
            commit,
            "trusted_corpus_missing",
            "trusted commit contains no Methexis records",
        ));
    }
    Ok(())
}

fn create_snapshot_root(
    operation: &'static str,
    commit: &str,
) -> Result<PathBuf, OperationFailure> {
    for _ in 0..16 {
        let sequence = SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "methexis-trusted-{}-{}-{sequence}",
            std::process::id(),
            &commit[..12]
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(error) => {
                return Err(failure(
                    operation,
                    commit,
                    "snapshot_materialization_failed",
                    &error.to_string(),
                ));
            },
        }
    }
    Err(failure(
        operation,
        commit,
        "snapshot_materialization_failed",
        "could not allocate a unique snapshot directory",
    ))
}

fn git_output(
    repository_root: &Path,
    args: &[&str],
    operation: &'static str,
    commit: Option<String>,
    code: &'static str,
) -> Result<Vec<u8>, OperationFailure> {
    let output = Command::new(SYSTEM_GIT)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_GRAFT_FILE", "/dev/null")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("LC_ALL", "C")
        .arg("--no-replace-objects")
        .arg("-C")
        .arg(repository_root)
        .args(args)
        .output()
        .map_err(|error| {
            OperationFailure::new(
                operation,
                commit.clone(),
                code,
                error.to_string(),
                Vec::new(),
                "repair the Git repository and retry",
            )
        })?;
    if !output.status.success() {
        return Err(OperationFailure::new(
            operation,
            commit,
            code,
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            Vec::new(),
            "repair the trusted Git ref or object database and retry",
        ));
    }
    Ok(output.stdout)
}

pub(super) fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn failure(
    operation: &'static str,
    commit: &str,
    code: &'static str,
    message: &str,
) -> OperationFailure {
    OperationFailure::new(
        operation,
        Some(commit.to_owned()),
        code,
        message,
        Vec::new(),
        "repair the trusted snapshot and retry",
    )
}
