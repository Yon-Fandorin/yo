//! Prospective Checkpoint authority used only to refresh registered context manifests.

use std::path::Path;

use super::{
    ACTIVATE_REQUEST_SCHEMA, ActivationRequest, ContextAuthority, DEFAULT_TRUSTED_REF,
    MAX_REQUEST_BYTES, OperationFailure, candidate, git, hash_bytes,
    records::{build_active, parse_active_bytes},
};
use crate::publication::{self, CapturedFile, PublicationError, TargetLock};

const OPERATION: &str = "refresh_context_manifests";

pub(crate) struct ProspectiveContext {
    pub(crate) authority: ContextAuthority,
    request: CapturedFile,
    checkpoint: CapturedFile,
    active: CapturedFile,
    _checkpoint_lock: TargetLock,
    _active_lock: TargetLock,
}

pub(crate) fn prepare_context_refresh(
    repository_root: &Path,
    request_path: &Path,
) -> Result<ProspectiveContext, OperationFailure> {
    let (request, request_capture) = read_request(repository_root, request_path)?;
    if request.schema != ACTIVATE_REQUEST_SCHEMA
        || !super::valid_hash(&request.checkpoint_id)
        || !super::valid_hash(&request.checkpoint_hash)
        || request
            .replace_active_hash
            .as_ref()
            .is_some_and(|hash| !super::valid_hash(hash))
    {
        return Err(failure(
            None,
            "invalid_activation_request",
            "activation request schema and hashes are invalid",
            vec![request.checkpoint_id],
            "use the exact request accepted by propose-activation",
        ));
    }

    let snapshot = git::resolve(repository_root, DEFAULT_TRUSTED_REF, OPERATION)?;
    let current_active_path = snapshot.root.join("methexis/active-checkpoint.yaml");
    let current_active_hash = if current_active_path.exists() {
        let (_, bytes) = super::records::read_active(&current_active_path, OPERATION)?;
        Some(hash_bytes(&bytes))
    } else {
        None
    };
    if request.replace_active_hash != current_active_hash {
        return Err(failure(
            Some(snapshot.commit.clone()),
            "active_checkpoint_compare_and_swap_mismatch",
            "activation request does not name the exact trusted active-record predecessor",
            vec![request.checkpoint_id],
            "regenerate the activation request from current develop",
        ));
    }

    let checkpoint_path = repository_root.join("methexis/checkpoints").join(format!(
        "{}.yaml",
        request
            .checkpoint_id
            .strip_prefix("sha256:")
            .expect("validated hash")
    ));
    let checkpoint_lock =
        publication::lock_target(repository_root, &checkpoint_path).map_err(|error| {
            publication_failure(Some(snapshot.commit.clone()), &request.checkpoint_id, error)
        })?;
    let checkpoint_capture = checkpoint_lock
        .capture(super::MAX_RECORD_BYTES)
        .map_err(|error| {
            failure(
                Some(snapshot.commit.clone()),
                "checkpoint_unreadable",
                error.to_string(),
                vec![request.checkpoint_id.clone()],
                "repair the prospective Checkpoint",
            )
        })?;
    let checkpoint_bytes = checkpoint_capture.bytes();
    let checkpoint = super::records::parse_checkpoint_bytes(checkpoint_bytes, OPERATION)?;
    let checkpoint_hash = hash_bytes(checkpoint_bytes);
    if checkpoint.checkpoint_id != request.checkpoint_id
        || checkpoint_hash != request.checkpoint_hash
        || checkpoint.trusted_commit != snapshot.commit
    {
        return Err(failure(
            Some(snapshot.commit.clone()),
            "checkpoint_mismatch",
            "activation request, prospective Checkpoint, and pinned develop do not match exactly",
            vec![request.checkpoint_id],
            "recreate the Checkpoint and activation proposal from current develop",
        ));
    }
    let active_path = repository_root.join("methexis/active-checkpoint.yaml");
    let active_lock = publication::lock_target(repository_root, &active_path).map_err(|error| {
        publication_failure(
            Some(snapshot.commit.clone()),
            &checkpoint.checkpoint_id,
            error,
        )
    })?;
    let active_capture = active_lock
        .capture(super::MAX_RECORD_BYTES)
        .map_err(|error| {
            failure(
                Some(snapshot.commit.clone()),
                "active_checkpoint_unreadable",
                error.to_string(),
                vec![checkpoint.checkpoint_id.clone()],
                "run propose-activation with this activation request",
            )
        })?;
    let active_bytes = active_capture.bytes();
    let active = parse_active_bytes(active_bytes, OPERATION)?;
    let (_, expected_active_bytes, _) = build_active(
        &checkpoint,
        &checkpoint_hash,
        current_active_hash.as_deref(),
    )?;
    if active_bytes != expected_active_bytes
        || active.checkpoint_id != request.checkpoint_id
        || active.checkpoint_hash != request.checkpoint_hash
    {
        return Err(failure(
            Some(snapshot.commit.clone()),
            "active_checkpoint_lineage_mismatch",
            "working active record is not the canonical proposal for this activation request",
            vec![request.checkpoint_id],
            "run propose-activation with this activation request",
        ));
    }

    let authority = candidate::build(
        repository_root,
        &snapshot,
        &checkpoint,
        checkpoint_bytes,
        active_bytes,
        OPERATION,
    )?;
    Ok(ProspectiveContext {
        authority,
        request: request_capture,
        checkpoint: checkpoint_capture,
        active: active_capture,
        _checkpoint_lock: checkpoint_lock,
        _active_lock: active_lock,
    })
}

impl ProspectiveContext {
    pub(crate) fn final_revalidate(&self, repository_root: &Path) -> Result<(), OperationFailure> {
        candidate::final_revalidate(
            repository_root,
            &self.authority,
            DEFAULT_TRUSTED_REF,
            OPERATION,
        )?;
        for (name, capture) in [
            ("request", &self.request),
            ("Checkpoint", &self.checkpoint),
            ("active record", &self.active),
        ] {
            if let Err(error) = capture.revalidate() {
                return Err(failure(
                    Some(self.authority.trusted_commit.clone()),
                    "activation_proposal_changed_during_refresh",
                    format!("captured activation {name} changed during manifest refresh: {error}"),
                    vec![self.authority.checkpoint_id.clone()],
                    "retry with the stable activation proposal",
                ));
            }
        }
        Ok(())
    }
}

fn read_request(
    repository_root: &Path,
    path: &Path,
) -> Result<(ActivationRequest, CapturedFile), OperationFailure> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        repository_root.join(path)
    };
    let capture = publication::capture_file(repository_root, &absolute, MAX_REQUEST_BYTES)
        .map_err(|error| publication_failure(None, "activation-request", error))?;
    let request = serde_json::from_slice(capture.bytes()).map_err(|error| {
        failure(
            None,
            "invalid_request",
            error.to_string(),
            Vec::new(),
            "repair the activation request",
        )
    })?;
    Ok((request, capture))
}

fn failure(
    commit: Option<String>,
    code: impl Into<String>,
    message: impl Into<String>,
    affected_ids: Vec<String>,
    next_action: impl Into<String>,
) -> OperationFailure {
    OperationFailure::new(OPERATION, commit, code, message, affected_ids, next_action)
}

fn publication_failure(
    commit: Option<String>,
    id: &str,
    error: PublicationError,
) -> OperationFailure {
    let (code, message) = match error {
        PublicationError::OutsideRepository => (
            "path_outside_repository",
            "activation proposal path escapes the repository".to_owned(),
        ),
        PublicationError::Symlink(path) => (
            "symlink_forbidden",
            format!("activation proposal path `{}` is a symlink", path.display()),
        ),
        PublicationError::NotDirectory(path) => (
            "publication_failed",
            format!(
                "activation proposal parent `{}` is not a directory",
                path.display()
            ),
        ),
        PublicationError::Locked(error) => ("publication_locked", error.to_string()),
        PublicationError::Io(error) | PublicationError::DurabilityUnknown(error) => {
            ("publication_failed", error.to_string())
        },
    };
    failure(
        commit,
        code,
        message,
        vec![id.to_owned()],
        "repair the activation proposal path or retry after the active writer finishes",
    )
}
