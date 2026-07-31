//! Deterministic Checkpoint and active-record encoding and validation.

use std::{fs, io::Read, path::Path};

use super::{
    ACTIVE_SCHEMA, ActiveIdentity, ActiveRecord, CHECKPOINT_SCHEMA, CheckpointIdentity,
    CheckpointRecord, MAX_RECORD_BYTES, OperationFailure, git::valid_commit, hash_bytes,
    semantic_hash, valid_hash,
};

pub(super) fn build_checkpoint(
    trusted_commit: &str,
    roots: Vec<String>,
    units: Vec<super::CheckpointUnit>,
) -> Result<(CheckpointRecord, Vec<u8>, String), OperationFailure> {
    const OPERATION: &str = "create_checkpoint";
    let checkpoint_id = semantic_hash(&CheckpointIdentity {
        schema: CHECKPOINT_SCHEMA,
        trusted_commit,
        source_status: "not_evaluated",
        roots: &roots,
        units: &units,
    });
    let record = CheckpointRecord {
        schema: CHECKPOINT_SCHEMA.to_owned(),
        checkpoint_id,
        trusted_commit: trusted_commit.to_owned(),
        source_status: "not_evaluated".to_owned(),
        roots,
        units,
    };
    let bytes = yaml_bytes(&record, OPERATION, Some(trusted_commit.to_owned()))?;
    let hash = hash_bytes(&bytes);
    Ok((record, bytes, hash))
}

pub(super) fn build_active(
    checkpoint: &CheckpointRecord,
    checkpoint_hash: &str,
    replaces_active_hash: Option<&str>,
) -> Result<(ActiveRecord, Vec<u8>, String), OperationFailure> {
    const OPERATION: &str = "propose_activation";
    let request_hash = semantic_hash(&ActiveIdentity {
        schema: ACTIVE_SCHEMA,
        checkpoint_id: &checkpoint.checkpoint_id,
        checkpoint_hash,
        trusted_commit: &checkpoint.trusted_commit,
        replaces_active_hash,
    });
    let record = ActiveRecord {
        schema: ACTIVE_SCHEMA.to_owned(),
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        checkpoint_hash: checkpoint_hash.to_owned(),
        trusted_commit: checkpoint.trusted_commit.clone(),
        replaces_active_hash: replaces_active_hash.map(str::to_owned),
        request_hash,
    };
    let bytes = yaml_bytes(&record, OPERATION, Some(checkpoint.trusted_commit.clone()))?;
    let hash = hash_bytes(&bytes);
    Ok((record, bytes, hash))
}

pub(super) fn read_checkpoint(
    path: &Path,
    operation: &'static str,
) -> Result<(CheckpointRecord, Vec<u8>), OperationFailure> {
    let bytes = read_bounded(path, operation, "checkpoint_unreadable")?;
    let record = parse_checkpoint_bytes(&bytes, operation)?;
    Ok((record, bytes))
}

pub(super) fn parse_checkpoint_bytes(
    bytes: &[u8],
    operation: &'static str,
) -> Result<CheckpointRecord, OperationFailure> {
    let record: CheckpointRecord = serde_norway::from_slice(bytes).map_err(|error| {
        failure(
            operation,
            None,
            "invalid_checkpoint",
            error.to_string(),
            "repair or recreate the immutable Checkpoint",
        )
    })?;
    validate_checkpoint(&record, operation)?;
    let canonical = yaml_bytes(&record, operation, Some(record.trusted_commit.clone()))?;
    if bytes != canonical {
        return Err(failure(
            operation,
            Some(record.trusted_commit),
            "checkpoint_canonical_mismatch",
            "Checkpoint bytes are not the deterministic canonical encoding",
            "recreate the Checkpoint",
        ));
    }
    Ok(record)
}

pub(super) fn parse_active_bytes(
    bytes: &[u8],
    operation: &'static str,
) -> Result<ActiveRecord, OperationFailure> {
    let record: ActiveRecord = serde_norway::from_slice(bytes).map_err(|error| {
        failure(
            operation,
            None,
            "invalid_active_checkpoint",
            error.to_string(),
            "repair the active record through review",
        )
    })?;
    if record.schema != ACTIVE_SCHEMA
        || !valid_hash(&record.checkpoint_id)
        || !valid_hash(&record.checkpoint_hash)
        || !valid_hash(&record.request_hash)
        || !valid_commit(&record.trusted_commit)
        || record
            .replaces_active_hash
            .as_ref()
            .is_some_and(|hash| !valid_hash(hash))
        || record.request_hash
            != semantic_hash(&ActiveIdentity {
                schema: ACTIVE_SCHEMA,
                checkpoint_id: &record.checkpoint_id,
                checkpoint_hash: &record.checkpoint_hash,
                trusted_commit: &record.trusted_commit,
                replaces_active_hash: record.replaces_active_hash.as_deref(),
            })
    {
        return Err(failure(
            operation,
            Some(record.trusted_commit),
            "active_checkpoint_lineage_mismatch",
            "active record fields do not match their deterministic lineage",
            "repair the active record through review",
        ));
    }
    let canonical = yaml_bytes(&record, operation, Some(record.trusted_commit.clone()))?;
    if bytes != canonical {
        return Err(failure(
            operation,
            Some(record.trusted_commit),
            "active_checkpoint_canonical_mismatch",
            "active record bytes are not the deterministic canonical encoding",
            "repair the active record through review",
        ));
    }
    Ok(record)
}

pub(super) fn read_active(
    path: &Path,
    operation: &'static str,
) -> Result<(ActiveRecord, Vec<u8>), OperationFailure> {
    let bytes = read_bounded(path, operation, "active_checkpoint_unreadable")?;
    let record = parse_active_bytes(&bytes, operation)?;
    Ok((record, bytes))
}

fn validate_checkpoint(
    record: &CheckpointRecord,
    operation: &'static str,
) -> Result<(), OperationFailure> {
    let mut roots = record.roots.clone();
    roots.sort();
    roots.dedup();
    let mut units = record.units.clone();
    units.sort_by(|left, right| left.id.cmp(&right.id));
    for unit in &mut units {
        unit.reasons.sort();
        unit.reasons.dedup();
    }
    let expected_id = semantic_hash(&CheckpointIdentity {
        schema: CHECKPOINT_SCHEMA,
        trusted_commit: &record.trusted_commit,
        source_status: "not_evaluated",
        roots: &roots,
        units: &units,
    });
    if record.schema != CHECKPOINT_SCHEMA
        || roots.is_empty()
        || record.source_status != "not_evaluated"
        || !valid_commit(&record.trusted_commit)
        || roots != record.roots
        || units != record.units
        || record.checkpoint_id != expected_id
        || record
            .units
            .iter()
            .any(|unit| !valid_hash(&unit.revision) || unit.reasons.is_empty())
    {
        return Err(failure(
            operation,
            Some(record.trusted_commit.clone()),
            "checkpoint_lineage_mismatch",
            "Checkpoint identity, ordering, or selected revisions are invalid",
            "recreate the Checkpoint from the trusted snapshot",
        ));
    }
    Ok(())
}

fn read_bounded(
    path: &Path,
    operation: &'static str,
    code: &'static str,
) -> Result<Vec<u8>, OperationFailure> {
    let mut file = fs::File::open(path).map_err(|error| {
        failure(
            operation,
            None,
            code,
            error.to_string(),
            "repair the record",
        )
    })?;
    let mut bytes = Vec::new();
    Read::take(&mut file, (MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            failure(
                operation,
                None,
                code,
                error.to_string(),
                "repair the record",
            )
        })?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(failure(
            operation,
            None,
            "checkpoint_record_too_large",
            "Checkpoint record exceeds the Pilot size limit",
            "reduce or repair the record",
        ));
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(failure(
            operation,
            None,
            "checkpoint_bom_forbidden",
            "Checkpoint records must not contain a UTF-8 BOM",
            "repair the record",
        ));
    }
    Ok(bytes)
}

fn yaml_bytes(
    value: &impl serde::Serialize,
    operation: &'static str,
    commit: Option<String>,
) -> Result<Vec<u8>, OperationFailure> {
    let mut bytes = serde_norway::to_string(value)
        .map_err(|error| {
            failure(
                operation,
                commit,
                "serialization_failed",
                error.to_string(),
                "report the compiler failure",
            )
        })?
        .into_bytes();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn failure(
    operation: &'static str,
    commit: Option<String>,
    code: &'static str,
    message: impl Into<String>,
    next_action: &'static str,
) -> OperationFailure {
    OperationFailure::new(operation, commit, code, message, Vec::new(), next_action)
}
