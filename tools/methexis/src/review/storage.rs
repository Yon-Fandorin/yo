//! Atomic publication, replacement preconditions, and path-safety checks.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{OperationFailure, failure_from_diagnostic, hash_bytes, records::parse_approval};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TargetLock {
    path: PathBuf,
}

impl Drop for TargetLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn publish_tracked(
    repository_root: &Path,
    target: &Path,
    bytes: &[u8],
    expected_existing_hash: Option<&str>,
    operation: &'static str,
    id: &str,
) -> Result<&'static str, OperationFailure> {
    ensure_safe_parent(repository_root, target, operation, id)?;
    reject_target_symlink(target, operation, id)?;
    let _lock = acquire_target_lock(target, operation, id)?;
    match fs::read(target) {
        Ok(existing) => {
            if existing == bytes {
                return Ok("unchanged");
            }
            let existing_hash = hash_bytes(&existing);
            if expected_existing_hash != Some(existing_hash.as_str()) {
                return Err(OperationFailure::new(
                    operation,
                    "replacement_conflict",
                    format!("existing Projection hash is `{existing_hash}`"),
                    vec![id.to_owned()],
                    "retry with replace_projection_hash set to the exact existing hash",
                ));
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if expected_existing_hash.is_some() {
                return Err(OperationFailure::new(
                    operation,
                    "replacement_conflict",
                    "no existing Projection is available for the replacement precondition",
                    vec![id.to_owned()],
                    "remove replace_projection_hash for initial creation",
                ));
            }
        },
        Err(error) => {
            return Err(OperationFailure::new(
                operation,
                "publication_failed",
                format!("cannot inspect existing Projection: {error}"),
                vec![id.to_owned()],
                "repair the destination and retry",
            ));
        },
    }
    atomic_write(target, bytes).map_err(|error| {
        OperationFailure::new(
            operation,
            "publication_failed",
            error.to_string(),
            vec![id.to_owned()],
            "repair the destination and retry the same request",
        )
    })?;
    Ok("written")
}

pub(super) fn publish_approval(
    repository_root: &Path,
    target: &Path,
    bytes: &[u8],
    expected_revision: Option<&str>,
    operation: &'static str,
    id: &str,
) -> Result<&'static str, OperationFailure> {
    ensure_safe_parent(repository_root, target, operation, id)?;
    reject_target_symlink(target, operation, id)?;
    let _lock = acquire_target_lock(target, operation, id)?;
    match fs::read(target) {
        Ok(existing) => {
            if existing == bytes {
                return Ok("unchanged");
            }
            let existing_record =
                parse_approval(target, repository_root).map_err(|diagnostic| {
                    failure_from_diagnostic(
                        operation,
                        diagnostic,
                        "repair or remove the damaged approval through review",
                    )
                })?;
            if expected_revision != Some(existing_record.revision.as_str()) {
                return Err(OperationFailure::new(
                    operation,
                    "approval_replacement_conflict",
                    format!(
                        "existing approval revision is `{}`",
                        existing_record.revision
                    ),
                    vec![id.to_owned()],
                    "retry with replace_revision set to the exact existing revision",
                ));
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if expected_revision.is_some() {
                return Err(OperationFailure::new(
                    operation,
                    "approval_replacement_conflict",
                    "no existing approval is available for the replacement precondition",
                    vec![id.to_owned()],
                    "remove replace_revision for initial creation",
                ));
            }
        },
        Err(error) => {
            return Err(OperationFailure::new(
                operation,
                "publication_failed",
                format!("cannot inspect existing approval: {error}"),
                vec![id.to_owned()],
                "repair the destination and retry",
            ));
        },
    }
    atomic_write(target, bytes).map_err(|error| {
        OperationFailure::new(
            operation,
            "publication_failed",
            error.to_string(),
            vec![id.to_owned()],
            "repair the destination and retry the same request",
        )
    })?;
    Ok("written")
}

fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| io::Error::other("target has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let temp = temporary_sibling(target);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn acquire_target_lock(
    target: &Path,
    operation: &'static str,
    id: &str,
) -> Result<TargetLock, OperationFailure> {
    let parent = target.parent().ok_or_else(|| {
        OperationFailure::new(
            operation,
            "publication_failed",
            "target has no parent directory",
            vec![id.to_owned()],
            "use the repository-local destination",
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        OperationFailure::new(
            operation,
            "publication_failed",
            error.to_string(),
            vec![id.to_owned()],
            "repair the destination and retry",
        )
    })?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("record");
    let path = target.with_file_name(format!(".{name}.lock"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            OperationFailure::new(
                operation,
                "publication_locked",
                error.to_string(),
                vec![id.to_owned()],
                "wait for the active writer or remove a confirmed stale lock",
            )
        })?;
    if let Err(error) = writeln!(file, "pid={}", std::process::id()) {
        let _ = fs::remove_file(&path);
        return Err(OperationFailure::new(
            operation,
            "publication_failed",
            error.to_string(),
            vec![id.to_owned()],
            "repair the destination and retry",
        ));
    }
    Ok(TargetLock { path })
}

pub(super) fn publish_artifact_directory(
    repository_root: &Path,
    target: &Path,
    files: &[(&str, &[u8])],
    operation: &'static str,
    id: &str,
) -> Result<&'static str, OperationFailure> {
    ensure_safe_parent(repository_root, target, operation, id)?;
    reject_target_symlink(target, operation, id)?;
    if target.exists() {
        let matches = files.iter().all(|(name, bytes)| {
            fs::read(target.join(name)).is_ok_and(|existing| existing == *bytes)
        });
        if matches {
            return Ok("unchanged");
        }
        return Err(OperationFailure::new(
            operation,
            "artifact_collision",
            "content-addressed review directory exists with different bytes",
            vec![id.to_owned()],
            "inspect the existing artifact and resolve corruption",
        ));
    }
    let parent = target.parent().ok_or_else(|| {
        OperationFailure::new(
            operation,
            "publication_failed",
            "review target has no parent",
            vec![id.to_owned()],
            "use the repository-local artifact root",
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        OperationFailure::new(
            operation,
            "publication_failed",
            error.to_string(),
            vec![id.to_owned()],
            "repair the local artifact root and retry",
        )
    })?;
    let temp = temporary_sibling(target);
    let result = (|| -> io::Result<()> {
        fs::create_dir(&temp)?;
        for (name, bytes) in files {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(temp.join(name))?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        fs::rename(&temp, target)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temp);
        return Err(OperationFailure::new(
            operation,
            "publication_failed",
            error.to_string(),
            vec![id.to_owned()],
            "repair the local artifact root and retry the same request",
        ));
    }
    Ok("written")
}

fn ensure_safe_parent(
    repository_root: &Path,
    target: &Path,
    operation: &'static str,
    id: &str,
) -> Result<(), OperationFailure> {
    let relative = target.strip_prefix(repository_root).map_err(|_| {
        OperationFailure::new(
            operation,
            "path_outside_repository",
            "output target escapes the repository root",
            vec![id.to_owned()],
            "use repository-local output paths",
        )
    })?;
    let components = relative.components().collect::<Vec<_>>();
    let mut current = repository_root.to_owned();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(OperationFailure::new(
                    operation,
                    "symlink_forbidden",
                    format!("output parent `{}` is a symlink", current.display()),
                    vec![id.to_owned()],
                    "replace the symlink with a repository-local directory",
                ));
            },
            Ok(metadata) if !metadata.is_dir() => {
                return Err(OperationFailure::new(
                    operation,
                    "publication_failed",
                    format!("output parent `{}` is not a directory", current.display()),
                    vec![id.to_owned()],
                    "repair the output parent and retry",
                ));
            },
            Ok(_) => {},
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => {
                return Err(OperationFailure::new(
                    operation,
                    "publication_failed",
                    error.to_string(),
                    vec![id.to_owned()],
                    "repair the output parent and retry",
                ));
            },
        }
    }
    Ok(())
}

fn reject_target_symlink(
    target: &Path,
    operation: &'static str,
    id: &str,
) -> Result<(), OperationFailure> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(OperationFailure::new(
            operation,
            "symlink_forbidden",
            format!("output target `{}` is a symlink", target.display()),
            vec![id.to_owned()],
            "replace the symlink with a repository-local path",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(OperationFailure::new(
            operation,
            "publication_failed",
            error.to_string(),
            vec![id.to_owned()],
            "repair the output target and retry",
        )),
    }
}

fn temporary_sibling(target: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    target.with_file_name(format!(".{name}.tmp-{}-{sequence}", std::process::id()))
}
