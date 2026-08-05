//! Immutable Checkpoint and active-record CAS publication policy.

use std::{io, path::Path};

use super::{OperationFailure, hash_bytes, records::parse_active_bytes};
use crate::publication::{self, PublicationError};

pub(super) fn publish_immutable(
    repository_root: &Path,
    target: &Path,
    bytes: &[u8],
    operation: &'static str,
    commit: &str,
    id: &str,
) -> Result<&'static str, OperationFailure> {
    let lock = publication::lock_target(repository_root, target)
        .map_err(|error| publication_failure(operation, commit, id, error))?;
    match lock.read() {
        Ok(existing) if existing == bytes => return Ok("unchanged"),
        Ok(_) => {
            return Err(failure(
                operation,
                commit,
                "checkpoint_collision",
                "Checkpoint destination exists with different bytes",
                id,
                "inspect and repair the immutable Checkpoint",
            ));
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        Err(error) => {
            return Err(failure(
                operation,
                commit,
                "publication_failed",
                error.to_string(),
                id,
                "repair the destination and retry",
            ));
        },
    }
    lock.atomic_create(bytes)
        .map_err(|error| publication_failure(operation, commit, id, error))?;
    Ok("written")
}

pub(super) fn publish_active(
    repository_root: &Path,
    target: &Path,
    bytes: &[u8],
    expected_hash: Option<&str>,
    operation: &'static str,
    commit: &str,
    id: &str,
) -> Result<&'static str, OperationFailure> {
    let lock = publication::lock_target(repository_root, target)
        .map_err(|error| publication_failure(operation, commit, id, error))?;
    let previous = match lock.read() {
        Ok(existing) if existing == bytes => return Ok("unchanged"),
        Ok(existing) => {
            parse_active_bytes(&existing, operation)?;
            let current_hash = hash_bytes(&existing);
            if expected_hash != Some(current_hash.as_str()) {
                return Err(failure(
                    operation,
                    commit,
                    "activation_conflict",
                    format!("current active-record hash is `{current_hash}`"),
                    id,
                    "rebuild the request from the current active record",
                ));
            }
            Some(existing)
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if expected_hash.is_some() {
                return Err(failure(
                    operation,
                    commit,
                    "activation_conflict",
                    "no active record exists for the replacement precondition",
                    id,
                    "remove replace_active_hash for initial activation",
                ));
            }
            None
        },
        Err(error) => {
            return Err(failure(
                operation,
                commit,
                "publication_failed",
                error.to_string(),
                id,
                "repair the destination and retry",
            ));
        },
    };
    lock.atomic_replace_or_restore(bytes, previous.as_deref())
        .map_err(|error| publication_failure(operation, commit, id, error))?;
    Ok("written")
}

fn publication_failure(
    operation: &'static str,
    commit: &str,
    id: &str,
    error: PublicationError,
) -> OperationFailure {
    let (code, message, next_action) = match error {
        PublicationError::OutsideRepository => (
            "path_outside_repository",
            "output target escapes the repository".to_owned(),
            "use the repository-local destination",
        ),
        PublicationError::Symlink(path) => (
            "symlink_forbidden",
            format!("output path `{}` is a symlink", path.display()),
            "replace it with a repository-local path",
        ),
        PublicationError::NotDirectory(path) => (
            "publication_failed",
            format!("output parent `{}` is not a directory", path.display()),
            "repair the destination",
        ),
        PublicationError::Locked(error) => (
            "publication_locked",
            error.to_string(),
            "wait for the active writer or remove a confirmed stale lock",
        ),
        PublicationError::DurabilityUnknown(error) => (
            "publication_recovery_required",
            format!("publication durability and rollback are uncertain: {error}"),
            "inspect the destination before retrying",
        ),
        PublicationError::Io(error) => (
            "publication_failed",
            error.to_string(),
            "repair the destination and retry",
        ),
    };
    failure(operation, commit, code, message, id, next_action)
}

fn failure(
    operation: &'static str,
    commit: &str,
    code: &'static str,
    message: impl Into<String>,
    id: &str,
    next_action: &'static str,
) -> OperationFailure {
    OperationFailure::new(
        operation,
        Some(commit.to_owned()),
        code,
        message,
        vec![id.to_owned()],
        next_action,
    )
}
