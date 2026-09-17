use std::{ffi::OsString, path::Path, process::ExitCode};

use super::{
    super::model::ContextResult, ContextCapture, ProspectiveCapture, activation, context, evidence,
};
use crate::review_protocol::Captured;

pub(in crate::review_packet) fn capture_prospective_context_with_request(
    repository: &Path,
    candidate_commit: &str,
    activation_request_path: &Path,
    activation_request: Captured,
    context_request_path: &Path,
    context_request: Captured,
) -> Result<(ContextCapture, ProspectiveCapture), String> {
    capture_prospective_context(
        repository,
        candidate_commit,
        activation_request_path,
        activation_request,
        context_request,
        || resolve_prospective_context(activation_request_path, context_request_path),
    )
}

fn resolve_prospective_context(
    activation_request_path: &Path,
    context_request_path: &Path,
) -> Result<ContextResult, String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let arguments = [
        OsString::from("resolve-activation-review-context"),
        activation_request_path.as_os_str().to_owned(),
        context_request_path.as_os_str().to_owned(),
    ];
    let code = methexis::run(arguments, &mut stdout, &mut stderr)
        .map_err(|error| format!("cannot run prospective Methexis ContextBuild: {error}"))?;
    if code != ExitCode::SUCCESS {
        return Err(format!(
            "prospective Methexis ContextBuild failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    serde_json::from_slice(&stdout)
        .map_err(|error| format!("invalid prospective Methexis ContextBuild result: {error}"))
}

#[cfg(test)]
pub(in crate::review_packet) fn capture_prospective_context_from_result(
    repository: &Path,
    candidate_commit: &str,
    activation_request_path: &Path,
    activation_request: Captured,
    context_request: Captured,
    result: ContextResult,
) -> Result<(ContextCapture, ProspectiveCapture), String> {
    capture_prospective_context(
        repository,
        candidate_commit,
        activation_request_path,
        activation_request,
        context_request,
        || Ok(result),
    )
}

fn capture_prospective_context(
    repository: &Path,
    candidate_commit: &str,
    activation_request_path: &Path,
    activation_request: Captured,
    context_request: Captured,
    resolve: impl FnOnce() -> Result<ContextResult, String>,
) -> Result<(ContextCapture, ProspectiveCapture), String> {
    let activation = activation::parse_activation_request(&activation_request.bytes)?;
    let checkpoint_path = format!(
        "methexis/checkpoints/{}.yaml",
        activation
            .checkpoint_id
            .strip_prefix("sha256:")
            .expect("validated activation CheckpointId")
    );
    let proposal = evidence::capture_authorities(
        repository,
        candidate_commit,
        &[
            checkpoint_path.clone(),
            "methexis/active-checkpoint.yaml".to_owned(),
        ],
    )?;
    let proposed_active_record = proposal
        .iter()
        .find(|capture| capture.path == "methexis/active-checkpoint.yaml")
        .cloned()
        .ok_or_else(|| "prospective active record capture is missing".to_owned())?;
    let proposed_checkpoint = proposal
        .iter()
        .find(|capture| capture.path == checkpoint_path)
        .cloned()
        .ok_or_else(|| "prospective Checkpoint capture is missing".to_owned())?;
    evidence::require_hash(
        &activation.checkpoint_hash,
        &proposed_checkpoint.bytes,
        "prospective Checkpoint",
    )?;
    let result = resolve()?;
    let context = context::capture_context_artifacts(repository, context_request, result)?;
    let result = &context.result;
    let checkpoint = result
        .checkpoint
        .as_ref()
        .ok_or_else(|| "prospective ContextBuild result omitted its Checkpoint".to_owned())?;
    let result_request = result.activation_request.as_ref().ok_or_else(|| {
        "prospective ContextBuild result omitted its activation request".to_owned()
    })?;
    if result.schema != "methexis.activation-review-context-result/v1alpha1"
        || !result.ok
        || result.operation != "resolve_activation_review_context"
        || result.authority != "prospective"
        || checkpoint != &context.active_checkpoint
        || checkpoint.id != activation.checkpoint_id
        || checkpoint.hash != activation.checkpoint_hash
        || checkpoint.authority_basis_commit != result.trusted_commit
        || result_request.hash != activation_request.hash
        || crate::review_protocol::resolve_input_path(repository, &result_request.path)
            != activation_request_path
        || result.predecessor_active_record_hash != activation.replace_active_hash
        || result.proposed_active_record_hash.as_deref()
            != Some(proposed_active_record.hash.as_str())
    {
        return Err(
            "prospective ContextBuild result, activation proposal, and manifest identities differ"
                .to_owned(),
        );
    }
    Ok((
        context,
        ProspectiveCapture {
            activation_request,
            proposed_checkpoint,
            proposed_active_record,
            predecessor_active_record_hash: activation.replace_active_hash,
        },
    ))
}
