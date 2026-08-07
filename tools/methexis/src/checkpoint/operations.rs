//! Checkpoint creation and activation-proposal orchestration.

use std::{fs, io::Read, path::Path};

use super::{
    ACTIVATE_REQUEST_SCHEMA, ActivationInput, ActivationRequest, CREATE_REQUEST_SCHEMA,
    CreateInput, CreateRequest, MAX_REQUEST_BYTES, OperationFailure, OperationSuccess, git,
    hash_bytes,
    records::{build_active, build_checkpoint, read_checkpoint},
    relative_path, semantic_hash,
    storage::{publish_active, publish_immutable},
    valid_hash, validation,
};

pub(super) fn create(
    repository_root: &Path,
    trusted_ref: &str,
    request_path: &Path,
) -> Result<OperationSuccess, OperationFailure> {
    const OPERATION: &str = "create_checkpoint";
    let request: CreateRequest = read_request(request_path, OPERATION)?;
    if request.schema != CREATE_REQUEST_SCHEMA {
        return Err(failure(
            OPERATION,
            None,
            "unsupported_request_schema",
            format!("expected `{CREATE_REQUEST_SCHEMA}`"),
            request.roots,
            "regenerate the versioned request",
        ));
    }
    let snapshot = git::resolve(repository_root, trusted_ref, OPERATION)?;
    let selected = validation::select(&snapshot, &request.roots, OPERATION)?;
    let request_hash = semantic_hash(&CreateInput {
        schema: CREATE_REQUEST_SCHEMA,
        roots: &selected.roots,
    });
    let (record, bytes, hash) = build_checkpoint(&snapshot.commit, selected.roots, selected.units)?;
    let filename = record
        .checkpoint_id
        .strip_prefix("sha256:")
        .expect("internally generated CheckpointId has a sha256 prefix");
    let target = repository_root
        .join("methexis/checkpoints")
        .join(format!("{filename}.yaml"));
    let status = publish_immutable(
        repository_root,
        &target,
        &bytes,
        OPERATION,
        &snapshot.commit,
        &record.checkpoint_id,
    )?;
    Ok(OperationSuccess {
        schema: super::OPERATION_SCHEMA,
        ok: true,
        operation: OPERATION,
        status,
        authority: "draft_proposal",
        trusted_commit: snapshot.commit.clone(),
        affected_ids: record.units.iter().map(|unit| unit.id.clone()).collect(),
        path: relative_path(repository_root, &target),
        hash,
        checkpoint_id: record.checkpoint_id,
        request_hash,
        next_actions: vec![
            "inspect the immutable Checkpoint before proposing activation".to_owned(),
        ],
    })
}

pub(super) fn propose_activation(
    repository_root: &Path,
    trusted_ref: &str,
    request_path: &Path,
) -> Result<OperationSuccess, OperationFailure> {
    const OPERATION: &str = "propose_activation";
    let request: ActivationRequest = read_request(request_path, OPERATION)?;
    if request.schema != ACTIVATE_REQUEST_SCHEMA {
        return Err(failure(
            OPERATION,
            None,
            "unsupported_request_schema",
            format!("expected `{ACTIVATE_REQUEST_SCHEMA}`"),
            vec![request.checkpoint_id],
            "regenerate the versioned request",
        ));
    }
    if !valid_hash(&request.checkpoint_id)
        || !valid_hash(&request.checkpoint_hash)
        || request
            .replace_active_hash
            .as_ref()
            .is_some_and(|hash| !valid_hash(hash))
    {
        return Err(failure(
            OPERATION,
            None,
            "invalid_activation_request",
            "Checkpoint ID, hash, and optional active predecessor must be lowercase sha256 values",
            vec![request.checkpoint_id],
            "use the exact values returned by create-checkpoint",
        ));
    }
    let request_hash = semantic_hash(&ActivationInput {
        schema: ACTIVATE_REQUEST_SCHEMA,
        checkpoint_id: &request.checkpoint_id,
        checkpoint_hash: &request.checkpoint_hash,
        replace_active_hash: request.replace_active_hash.as_deref(),
    });
    let filename = request
        .checkpoint_id
        .strip_prefix("sha256:")
        .expect("validated CheckpointId has a sha256 prefix");
    let checkpoint_path = repository_root
        .join("methexis/checkpoints")
        .join(format!("{filename}.yaml"));
    let (checkpoint, checkpoint_bytes) = read_checkpoint(&checkpoint_path, OPERATION)?;
    let snapshot = git::resolve(repository_root, trusted_ref, OPERATION)?;
    if checkpoint.trusted_commit != snapshot.commit {
        return Err(failure(
            OPERATION,
            Some(snapshot.commit.clone()),
            "checkpoint_trust_mismatch",
            "Checkpoint was not created from the current trusted commit",
            vec![request.checkpoint_id],
            "recreate the Checkpoint from the current trusted integration",
        ));
    }
    validation::verify_lineage(&snapshot, &checkpoint, &checkpoint_bytes, OPERATION)?;
    let actual_hash = hash_bytes(&checkpoint_bytes);
    if checkpoint.checkpoint_id != request.checkpoint_id || actual_hash != request.checkpoint_hash {
        return Err(failure(
            OPERATION,
            Some(checkpoint.trusted_commit),
            "checkpoint_mismatch",
            "activation request does not match the exact immutable Checkpoint",
            vec![request.checkpoint_id],
            "use the exact Checkpoint ID and hash",
        ));
    }
    let (_, bytes, hash) = build_active(
        &checkpoint,
        &actual_hash,
        request.replace_active_hash.as_deref(),
    )?;
    let target = repository_root.join("methexis/active-checkpoint.yaml");
    let status = publish_active(
        repository_root,
        &target,
        &bytes,
        request.replace_active_hash.as_deref(),
        OPERATION,
        &checkpoint.trusted_commit,
        &checkpoint.checkpoint_id,
    )?;
    Ok(OperationSuccess {
        schema: super::OPERATION_SCHEMA,
        ok: true,
        operation: OPERATION,
        status,
        authority: "draft_proposal",
        trusted_commit: checkpoint.trusted_commit,
        affected_ids: checkpoint
            .units
            .iter()
            .map(|unit| unit.id.clone())
            .collect(),
        path: relative_path(repository_root, &target),
        hash,
        checkpoint_id: checkpoint.checkpoint_id,
        request_hash,
        next_actions: vec![
            "submit the Checkpoint and active record through repository review".to_owned(),
        ],
    })
}

pub(super) fn read_request<T>(path: &Path, operation: &'static str) -> Result<T, OperationFailure>
where
    T: serde::de::DeserializeOwned,
{
    let mut file = fs::File::open(path).map_err(|error| {
        failure(
            operation,
            None,
            "request_unreadable",
            error.to_string(),
            Vec::new(),
            "provide a readable versioned JSON request",
        )
    })?;
    let mut bytes = Vec::new();
    Read::take(&mut file, (MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            failure(
                operation,
                None,
                "request_unreadable",
                error.to_string(),
                Vec::new(),
                "provide a readable versioned JSON request",
            )
        })?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(failure(
            operation,
            None,
            "request_too_large",
            "request exceeds the Pilot size limit",
            Vec::new(),
            "reduce the request to required fields",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        failure(
            operation,
            None,
            "invalid_request",
            error.to_string(),
            Vec::new(),
            "repair the versioned JSON request",
        )
    })
}

fn failure(
    operation: &'static str,
    commit: Option<String>,
    code: &'static str,
    message: impl Into<String>,
    affected_ids: Vec<String>,
    next_action: &'static str,
) -> OperationFailure {
    OperationFailure::new(operation, commit, code, message, affected_ids, next_action)
}
