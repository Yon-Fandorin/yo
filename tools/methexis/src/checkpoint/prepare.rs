//! Checkpoint and activation request preparation from current authority state.
//!
//! `prepare-checkpoint` binds the active Checkpoint's roots into the exact
//! CreateRequest wire shape, and `prepare-activation` binds one
//! `create-checkpoint` result plus the working active-record hash into the
//! exact ActivationRequest wire shape. Both emit proposal request JSON only;
//! neither invokes Checkpoint creation or activation internally.

use std::{fs, io::ErrorKind, path::Path};

use serde::Deserialize;

use super::{
    ACTIVATE_REQUEST_SCHEMA, ActivationRequest, CREATE_REQUEST_SCHEMA, CreateRequest,
    OPERATION_SCHEMA, OperationFailure, hash_bytes,
    operations::read_request,
    records::{read_active, read_checkpoint},
    valid_hash,
};

/// The fields of one `create-checkpoint` result that activation preparation
/// consumes. Extra result fields are ignored.
#[derive(Clone, Debug, Deserialize)]
struct CreateOutput {
    schema: String,
    ok: bool,
    operation: String,
    checkpoint_id: String,
    hash: String,
}

pub(super) fn prepare_checkpoint(
    repository_root: &Path,
) -> Result<CreateRequest, OperationFailure> {
    const OPERATION: &str = "prepare_checkpoint";
    let active_path = repository_root.join("methexis/active-checkpoint.yaml");
    if let Err(error) = fs::metadata(&active_path)
        && error.kind() == ErrorKind::NotFound
    {
        return Err(OperationFailure::new(
            OPERATION,
            None,
            "no_active_checkpoint",
            "no active Checkpoint exists in the working tree",
            Vec::new(),
            "integrate an activation before preparing the next Checkpoint",
        ));
    }
    let (active, _) = read_active(&active_path, OPERATION)?;
    let filename = active
        .checkpoint_id
        .strip_prefix("sha256:")
        .expect("validated active record carries a sha256 CheckpointId");
    let checkpoint_path = repository_root
        .join("methexis/checkpoints")
        .join(format!("{filename}.yaml"));
    let (checkpoint, _) = read_checkpoint(&checkpoint_path, OPERATION)?;
    Ok(CreateRequest {
        schema: CREATE_REQUEST_SCHEMA.to_owned(),
        roots: checkpoint.roots,
    })
}

pub(super) fn prepare_activation(
    repository_root: &Path,
    output_path: &Path,
) -> Result<ActivationRequest, OperationFailure> {
    const OPERATION: &str = "prepare_activation";
    let output: CreateOutput = read_request(output_path, OPERATION)?;
    if output.schema != OPERATION_SCHEMA
        || !output.ok
        || output.operation != "create_checkpoint"
        || !valid_hash(&output.checkpoint_id)
        || !valid_hash(&output.hash)
    {
        return Err(OperationFailure::new(
            OPERATION,
            None,
            "invalid_create_output",
            "input is not a successful create-checkpoint operation result",
            vec![output.checkpoint_id],
            "save the stdout of `methexis create-checkpoint` as the input JSON",
        ));
    }
    let active_path = repository_root.join("methexis/active-checkpoint.yaml");
    let replace_active_hash = match fs::read(&active_path) {
        Ok(bytes) => Some(hash_bytes(&bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(OperationFailure::new(
                OPERATION,
                None,
                "active_checkpoint_unreadable",
                error.to_string(),
                Vec::new(),
                "repair the active record",
            ));
        },
    };
    Ok(ActivationRequest {
        schema: ACTIVATE_REQUEST_SCHEMA.to_owned(),
        checkpoint_id: output.checkpoint_id,
        checkpoint_hash: output.hash,
        replace_active_hash,
    })
}
