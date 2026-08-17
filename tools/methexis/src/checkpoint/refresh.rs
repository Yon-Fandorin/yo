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
    operation: &'static str,
    review_request_path: Option<String>,
    request_hash: String,
    predecessor_active_hash: Option<String>,
    request: CapturedFile,
    checkpoint: CapturedFile,
    active: CapturedFile,
    _checkpoint_lock: TargetLock,
    _active_lock: TargetLock,
}

#[derive(Clone, Copy)]
enum ActivationArtifact {
    Request,
    Checkpoint,
    ActiveRecord,
}

impl ActivationArtifact {
    fn label(self) -> &'static str {
        match self {
            Self::Request => "activation request",
            Self::Checkpoint => "prospective Checkpoint",
            Self::ActiveRecord => "active record",
        }
    }

    fn repair_action(self) -> &'static str {
        match self {
            Self::Request => {
                "repair the activation request path or retry after the active writer finishes"
            },
            Self::Checkpoint => {
                "repair the prospective Checkpoint path or retry after the active writer finishes"
            },
            Self::ActiveRecord => {
                "repair the active record path or retry after the active writer finishes"
            },
        }
    }
}

pub(crate) fn prepare_context_refresh(
    repository_root: &Path,
    request_path: &Path,
) -> Result<ProspectiveContext, OperationFailure> {
    prepare_prospective_context(repository_root, request_path, OPERATION)
}

pub(crate) fn prepare_prospective_context(
    repository_root: &Path,
    request_path: &Path,
    operation: &'static str,
) -> Result<ProspectiveContext, OperationFailure> {
    let (request, request_capture) = read_request(repository_root, request_path, operation)?;
    if request.schema != ACTIVATE_REQUEST_SCHEMA
        || !super::valid_hash(&request.checkpoint_id)
        || !super::valid_hash(&request.checkpoint_hash)
        || request
            .replace_active_hash
            .as_ref()
            .is_some_and(|hash| !super::valid_hash(hash))
    {
        return Err(failure_for(
            operation,
            None,
            "invalid_activation_request",
            "activation request schema and hashes are invalid",
            vec![request.checkpoint_id],
            "use the exact request accepted by propose-activation",
        ));
    }

    let snapshot = git::resolve(repository_root, DEFAULT_TRUSTED_REF, operation)?;
    let current_active_path = snapshot.root.join("methexis/active-checkpoint.yaml");
    let current_active_hash = if current_active_path.exists() {
        let (_, bytes) = super::records::read_active(&current_active_path, operation)?;
        Some(hash_bytes(&bytes))
    } else {
        None
    };
    if request.replace_active_hash != current_active_hash {
        return Err(failure_for(
            operation,
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
            publication_failure(
                operation,
                Some(snapshot.commit.clone()),
                &request.checkpoint_id,
                ActivationArtifact::Checkpoint,
                error,
            )
        })?;
    let checkpoint_capture = checkpoint_lock
        .capture(super::MAX_RECORD_BYTES)
        .map_err(|error| {
            failure_for(
                operation,
                Some(snapshot.commit.clone()),
                "checkpoint_unreadable",
                error.to_string(),
                vec![request.checkpoint_id.clone()],
                "repair the prospective Checkpoint",
            )
        })?;
    let checkpoint_bytes = checkpoint_capture.bytes();
    let checkpoint = super::records::parse_checkpoint_bytes(checkpoint_bytes, operation)?;
    let checkpoint_hash = hash_bytes(checkpoint_bytes);
    if checkpoint.checkpoint_id != request.checkpoint_id
        || checkpoint_hash != request.checkpoint_hash
        || checkpoint.trusted_commit != snapshot.commit
    {
        return Err(failure_for(
            operation,
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
            operation,
            Some(snapshot.commit.clone()),
            &checkpoint.checkpoint_id,
            ActivationArtifact::ActiveRecord,
            error,
        )
    })?;
    let active_capture = active_lock
        .capture(super::MAX_RECORD_BYTES)
        .map_err(|error| {
            failure_for(
                operation,
                Some(snapshot.commit.clone()),
                "active_checkpoint_unreadable",
                error.to_string(),
                vec![checkpoint.checkpoint_id.clone()],
                "run propose-activation with this activation request",
            )
        })?;
    let active_bytes = active_capture.bytes();
    let active = parse_active_bytes(active_bytes, operation)?;
    let (_, expected_active_bytes, _) = build_active(
        &checkpoint,
        &checkpoint_hash,
        current_active_hash.as_deref(),
    )?;
    if active_bytes != expected_active_bytes
        || active.checkpoint_id != request.checkpoint_id
        || active.checkpoint_hash != request.checkpoint_hash
    {
        return Err(failure_for(
            operation,
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
        operation,
    )?;
    let request_target = if request_path.is_absolute() {
        request_path.to_owned()
    } else {
        repository_root.join(request_path)
    };
    let review_request_path = if operation == OPERATION {
        None
    } else {
        Some(
            request_target
                .strip_prefix(repository_root)
                .unwrap_or(&request_target)
                .to_str()
                .ok_or_else(|| {
                    failure_for(
                        operation,
                        Some(snapshot.commit.clone()),
                        "activation_request_path_not_utf8",
                        "activation request path is not UTF-8 and cannot be bound into review evidence",
                        vec![request.checkpoint_id.clone()],
                        "move the activation request to a UTF-8 path inside the repository",
                    )
                })?
                .replace('\\', "/"),
        )
    };
    Ok(ProspectiveContext {
        authority,
        operation,
        review_request_path,
        request_hash: hash_bytes(request_capture.bytes()),
        predecessor_active_hash: request.replace_active_hash,
        request: request_capture,
        checkpoint: checkpoint_capture,
        active: active_capture,
        _checkpoint_lock: checkpoint_lock,
        _active_lock: active_lock,
    })
}

impl ProspectiveContext {
    pub(crate) fn request_path(&self) -> &str {
        self.review_request_path
            .as_deref()
            .expect("review-only context records its UTF-8 activation request path")
    }

    pub(crate) fn request_hash(&self) -> &str {
        &self.request_hash
    }

    pub(crate) fn predecessor_active_hash(&self) -> Option<&str> {
        self.predecessor_active_hash.as_deref()
    }

    pub(crate) fn proposed_active_record_hash(&self) -> &str {
        &self.authority.active_record_hash
    }

    pub(crate) fn final_revalidate(&self, repository_root: &Path) -> Result<(), OperationFailure> {
        candidate::final_revalidate(
            repository_root,
            &self.authority,
            DEFAULT_TRUSTED_REF,
            self.operation,
        )?;
        for (name, capture) in [
            ("request", &self.request),
            ("Checkpoint", &self.checkpoint),
            ("active record", &self.active),
        ] {
            if let Err(error) = capture.revalidate() {
                let (code, message, next_action) = if self.operation == OPERATION {
                    (
                        "activation_proposal_changed_during_refresh",
                        format!(
                            "captured activation {name} changed during manifest refresh: {error}"
                        ),
                        "retry with the stable activation proposal",
                    )
                } else {
                    (
                        "activation_proposal_changed_during_review_context",
                        format!(
                            "captured activation {name} changed during review ContextBuild: {error}"
                        ),
                        "retry the review after the activation proposal stops changing",
                    )
                };
                return Err(failure_for(
                    self.operation,
                    Some(self.authority.trusted_commit.clone()),
                    code,
                    message,
                    vec![self.authority.checkpoint_id.clone()],
                    next_action,
                ));
            }
        }
        Ok(())
    }
}

fn read_request(
    repository_root: &Path,
    path: &Path,
    operation: &'static str,
) -> Result<(ActivationRequest, CapturedFile), OperationFailure> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        repository_root.join(path)
    };
    let capture = publication::capture_file(repository_root, &absolute, MAX_REQUEST_BYTES)
        .map_err(|error| {
            publication_failure(
                operation,
                None,
                "activation-request",
                ActivationArtifact::Request,
                error,
            )
        })?;
    let request = serde_json::from_slice(capture.bytes()).map_err(|error| {
        failure_for(
            operation,
            None,
            "invalid_request",
            error.to_string(),
            Vec::new(),
            "repair the activation request",
        )
    })?;
    Ok((request, capture))
}

fn failure_for(
    operation: &'static str,
    commit: Option<String>,
    code: impl Into<String>,
    message: impl Into<String>,
    affected_ids: Vec<String>,
    next_action: impl Into<String>,
) -> OperationFailure {
    OperationFailure::new(operation, commit, code, message, affected_ids, next_action)
}

fn publication_failure(
    operation: &'static str,
    commit: Option<String>,
    id: &str,
    artifact: ActivationArtifact,
    error: PublicationError,
) -> OperationFailure {
    let label = artifact.label();
    let (code, message) = match error {
        PublicationError::OutsideRepository => (
            "path_outside_repository",
            format!("{label} path escapes the repository"),
        ),
        PublicationError::Symlink(path) => (
            "symlink_forbidden",
            format!("{label} path `{}` is a symlink", path.display()),
        ),
        PublicationError::NotDirectory(path) => (
            "publication_failed",
            format!("{label} parent `{}` is not a directory", path.display()),
        ),
        PublicationError::Locked(error) => (
            "publication_locked",
            format!("{label} publication is locked: {error}"),
        ),
        PublicationError::Io(error) | PublicationError::DurabilityUnknown(error) => (
            "publication_failed",
            format!("{label} publication failed: {error}"),
        ),
    };
    failure_for(
        operation,
        commit,
        code,
        message,
        vec![id.to_owned()],
        artifact.repair_action(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{ActivationArtifact, OPERATION, publication_failure};
    use crate::publication::PublicationError;

    // request, prospective Checkpoint, active record 각각에 대해 repository escape,
    // symlink, nondirectory-parent 분류가 code를 유지하면서 실제 subject를 message와
    // next_action 끝까지 보존합니다.
    #[test]
    fn publication_path_failures_name_the_exact_activation_artifact() {
        for (artifact, subject, id) in [
            (
                ActivationArtifact::Request,
                "activation request",
                "activation-request",
            ),
            (
                ActivationArtifact::Checkpoint,
                "prospective Checkpoint",
                "sha256:checkpoint",
            ),
            (
                ActivationArtifact::ActiveRecord,
                "active record",
                "sha256:checkpoint",
            ),
        ] {
            for (error, expected_code) in [
                (
                    PublicationError::OutsideRepository,
                    "path_outside_repository",
                ),
                (
                    PublicationError::Symlink(PathBuf::from("artifact")),
                    "symlink_forbidden",
                ),
                (
                    PublicationError::NotDirectory(PathBuf::from("parent")),
                    "publication_failed",
                ),
            ] {
                let failure = publication_failure(OPERATION, None, id, artifact, error);
                let value = serde_json::to_value(failure).unwrap();

                assert_eq!(value["error"]["code"], expected_code);
                assert!(
                    value["error"]["message"]
                        .as_str()
                        .unwrap()
                        .contains(subject)
                );
                assert_eq!(value["error"]["affected_ids"], json!([id]));
                assert!(
                    value["error"]["next_actions"][0]
                        .as_str()
                        .unwrap()
                        .contains(subject)
                );
            }
        }
    }
}
