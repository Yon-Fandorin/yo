//! Atomic publication, replacement preconditions, and path-safety checks.

use std::{io, path::Path};

use super::{OperationFailure, failure_from_diagnostic, hash_bytes, records::parse_approval_bytes};
use crate::publication::{self, DirectoryState, PublicationError};

pub(super) fn publish_tracked(
    repository_root: &Path,
    target: &Path,
    bytes: &[u8],
    expected_existing_hash: Option<&str>,
    operation: &'static str,
    id: &str,
) -> Result<&'static str, OperationFailure> {
    let lock = publication::lock_target(repository_root, target)
        .map_err(|error| publication_failure(operation, id, error))?;
    match lock.read() {
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
    lock.atomic_write(bytes)
        .map_err(|error| publication_failure(operation, id, error))?;
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
    let lock = publication::lock_target(repository_root, target)
        .map_err(|error| publication_failure(operation, id, error))?;
    match lock.read() {
        Ok(existing) => {
            if existing == bytes {
                return Ok("unchanged");
            }
            let existing_record = parse_approval_bytes(&existing, target, repository_root)
                .map_err(|diagnostic| {
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
    lock.atomic_write(bytes)
        .map_err(|error| publication_failure(operation, id, error))?;
    Ok("written")
}

pub(super) fn publish_artifact_directory(
    repository_root: &Path,
    target: &Path,
    files: &[(&str, &[u8])],
    operation: &'static str,
    id: &str,
) -> Result<&'static str, OperationFailure> {
    let lock = publication::lock_target(repository_root, target)
        .map_err(|error| publication_failure(operation, id, error))?;
    match lock
        .directory_state(files)
        .map_err(|error| publication_failure(operation, id, error))?
    {
        DirectoryState::Matches => return Ok("unchanged"),
        DirectoryState::Different => {
            return Err(OperationFailure::new(
                operation,
                "artifact_collision",
                "content-addressed review directory exists with different bytes",
                vec![id.to_owned()],
                "inspect the existing artifact and resolve corruption",
            ));
        },
        DirectoryState::Missing => {},
    }
    lock.atomic_create_directory(files).map_err(|error| {
        if matches!(&error, PublicationError::Io(inner) if inner.kind() == io::ErrorKind::AlreadyExists)
        {
            OperationFailure::new(
                operation,
                "artifact_collision",
                "content-addressed review directory appeared during publication",
                vec![id.to_owned()],
                "inspect the existing artifact and retry",
            )
        } else {
            publication_failure(operation, id, error)
        }
    })?;
    Ok("written")
}

fn publication_failure(
    operation: &'static str,
    id: &str,
    error: PublicationError,
) -> OperationFailure {
    let (code, message, next_action) = match error {
        PublicationError::OutsideRepository => (
            "path_outside_repository",
            "output target escapes the repository root".to_owned(),
            "use repository-local output paths",
        ),
        PublicationError::Symlink(path) => (
            "symlink_forbidden",
            format!("output path `{}` is a symlink", path.display()),
            "replace the symlink with a repository-local path",
        ),
        PublicationError::NotDirectory(path) => (
            "publication_failed",
            format!("output parent `{}` is not a directory", path.display()),
            "repair the output parent and retry",
        ),
        PublicationError::Locked(error) => (
            "publication_locked",
            error.to_string(),
            "wait for the active writer or remove a confirmed stale lock",
        ),
        PublicationError::Io(error) => (
            "publication_failed",
            error.to_string(),
            "repair the destination and retry",
        ),
    };
    OperationFailure::new(operation, code, message, vec![id.to_owned()], next_action)
}
