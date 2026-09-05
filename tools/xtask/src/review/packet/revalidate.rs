use std::path::Path;

use super::{
    MAX_INPUT_BYTES,
    canonical::delivery_profile_bytes_for_id,
    capture::{
        Inputs, capture_authorities, capture_context, capture_diff,
        capture_prospective_context_with_request, capture_validation, same_capture, same_captures,
        same_named_captures,
    },
    model::Request,
    trusted_git::{trusted_ensure_clean, trusted_resolve_commit},
};
use crate::{
    bounded_file,
    review_protocol::{Captured, NamedCaptured, digest, resolve_input_path},
    slice_contract,
};

pub(super) fn final_revalidate(
    repository: &Path,
    request: &Request,
    inputs: &Inputs,
) -> Result<(), String> {
    trusted_ensure_clean(
        repository,
        "candidate worktree",
        "returning a review packet",
    )?;
    if trusted_resolve_commit(repository, "HEAD")? != inputs.candidate_commit {
        return Err("candidate HEAD changed during review packet construction".to_owned());
    }
    if trusted_resolve_commit(repository, "refs/heads/develop")?
        != inputs.context.result.trusted_commit
    {
        return Err("trusted integration changed during review packet construction".to_owned());
    }
    if capture_diff(repository, &inputs.base_commit, &inputs.candidate_commit)? != inputs.diff.bytes
    {
        return Err("base-to-candidate diff changed during review packet construction".to_owned());
    }
    let authorities = capture_authorities(
        repository,
        &inputs.candidate_commit,
        &request.repository_authority_paths,
    )?;
    require_captures(&authorities, &inputs.authorities, "repository authority")?;
    let contract_path = resolve_input_path(repository, &request.slice_contract_path);
    require_current_file(&contract_path, &inputs.slice_contract, "Slice contract")?;
    let bound = slice_contract::trusted_bound_slice(repository)?;
    let canonical_contract = std::fs::canonicalize(&contract_path)
        .map_err(|error| format!("cannot resolve Slice contract: {error}"))?;
    if bound.contract_path != canonical_contract
        || bound.base != inputs.base_commit
        || bound.contract_id != inputs.slice_contract.hash
    {
        return Err("bound Slice contract identity changed".to_owned());
    }
    let validation = capture_validation(
        repository,
        &inputs.candidate_commit,
        &request.validation_evidence,
    )?;
    require_named_captures(&validation, &inputs.validation)?;
    let context_request_path = resolve_input_path(repository, &request.context_request_path);
    require_current_file(
        &context_request_path,
        &inputs.context.request,
        "ContextBuild request",
    )?;
    let current = if let Some(expected_proposal) = &inputs.prospective {
        let activation_request_path = resolve_input_path(
            repository,
            request.activation_request_path.as_deref().ok_or_else(|| {
                "prospective review request lost activation_request_path".to_owned()
            })?,
        );
        require_current_file(
            &activation_request_path,
            &expected_proposal.activation_request,
            "activation request",
        )?;
        let (current, proposal) = capture_prospective_context_with_request(
            repository,
            &inputs.candidate_commit,
            &activation_request_path,
            expected_proposal.activation_request.clone(),
            &context_request_path,
            inputs.context.request.clone(),
        )?;
        if !same_capture(
            &proposal.proposed_checkpoint,
            &expected_proposal.proposed_checkpoint,
        ) || !same_capture(
            &proposal.proposed_active_record,
            &expected_proposal.proposed_active_record,
        ) || proposal.predecessor_active_record_hash
            != expected_proposal.predecessor_active_record_hash
        {
            return Err(
                "prospective activation proposal changed during review packet construction"
                    .to_owned(),
            );
        }
        current
    } else {
        capture_context(repository, &context_request_path)?
    };
    if current.result.trusted_commit != inputs.context.result.trusted_commit
        || current.result.build_id != inputs.context.result.build_id
        || current.result.context != inputs.context.result.context
        || current.result.manifest != inputs.context.result.manifest
        || current.active_checkpoint != inputs.context.active_checkpoint
        || current.included_ids != inputs.context.included_ids
        || current.context.bytes != inputs.context.context.bytes
        || current.manifest.bytes != inputs.context.manifest.bytes
    {
        return Err("ContextBuild identity, freshness, or artifact bytes changed".to_owned());
    }
    if delivery_profile_bytes_for_id(&request.delivery_profile)? != inputs.delivery_profile_bytes {
        return Err("delivery profile bytes changed during review packet construction".to_owned());
    }
    Ok(())
}

fn require_current_file(path: &Path, expected: &Captured, label: &str) -> Result<(), String> {
    let bytes = bounded_file::read_regular(path, MAX_INPUT_BYTES, label)?;
    if digest(&bytes) == expected.hash && bytes == expected.bytes {
        Ok(())
    } else {
        Err(format!("{label} changed during review packet construction"))
    }
}

fn require_captures(actual: &[Captured], expected: &[Captured], label: &str) -> Result<(), String> {
    if same_captures(actual, expected) {
        Ok(())
    } else {
        Err(format!(
            "{label} inputs changed during review packet construction"
        ))
    }
}

fn require_named_captures(
    actual: &[NamedCaptured],
    expected: &[NamedCaptured],
) -> Result<(), String> {
    if same_named_captures(actual, expected) {
        Ok(())
    } else {
        Err("validation evidence changed during review packet construction".to_owned())
    }
}
