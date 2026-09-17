use std::path::Path;

use serde::Deserialize;

use super::super::{DIAGNOSTIC_LIMIT, REQUEST_LIMIT, REVIEW_RESULT_LIMIT};
use crate::{bounded_file, review_protocol::digest};

#[derive(Debug, Deserialize)]
pub(super) struct StoredArtifact {
    path: String,
    hash: String,
    bytes: usize,
    published: bool,
}

pub(super) fn read_json(path: &Path, label: &str) -> Result<serde_json::Value, String> {
    let bytes = bounded_file::read_regular(path, REQUEST_LIMIT, label)?;
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid {label}: {error}"))
}

pub(super) fn validate_process_and_artifacts(
    output: &Path,
    request_id: &str,
    continuation: bool,
) -> Result<(), String> {
    let outcome = read_json(&output.join("outcome.json"), "delegated delivery outcome")?;
    let expected_schema = if continuation {
        "yo.external-review-delegated-continuation-delivery-outcome/v1alpha1"
    } else {
        "yo.external-review-delegated-delivery-outcome/v1alpha1"
    };
    if outcome.get("schema").and_then(serde_json::Value::as_str) != Some(expected_schema)
        || outcome
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            != Some(request_id)
        || outcome
            .pointer("/process/exit_code")
            .and_then(serde_json::Value::as_i64)
            != Some(0)
        || outcome
            .pointer("/process/signal")
            .is_some_and(|value| !value.is_null())
    {
        return Err(
            "delegated delivery finalization requires an immutable successful process outcome"
                .to_owned(),
        );
    }
    verify_artifact(
        output,
        outcome
            .get("review_result")
            .ok_or_else(|| "delegated outcome has no review_result".to_owned())?,
        "review.txt",
        REVIEW_RESULT_LIMIT,
        "delegated review result",
    )?;
    verify_artifact(
        output,
        outcome
            .get("diagnostic")
            .ok_or_else(|| "delegated outcome has no diagnostic".to_owned())?,
        "diagnostic.txt",
        DIAGNOSTIC_LIMIT,
        "delegated review diagnostic",
    )?;
    Ok(())
}

pub(super) fn verify_artifact(
    output: &Path,
    value: &serde_json::Value,
    file: &str,
    limit: usize,
    label: &str,
) -> Result<(), String> {
    let artifact: StoredArtifact = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid {label} artifact: {error}"))?;
    let expected_path = output.join(file);
    if !artifact.published || artifact.path != expected_path.to_string_lossy() {
        return Err(format!(
            "{label} was not published at its exact output path"
        ));
    }
    let bytes = bounded_file::read_regular(&expected_path, limit, label)?;
    if artifact.bytes != bytes.len() || artifact.hash != digest(&bytes) {
        return Err(format!("{label} bytes differ from the immutable outcome"));
    }
    Ok(())
}
