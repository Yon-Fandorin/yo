//! Review-only ContextBuild compilation against one exact activation proposal.

use std::path::Path;

use super::{
    operations, storage,
    wire::{ProspectiveResolveSuccess, ProspectiveSuccessInput, ResolveFailure},
};
use crate::{checkpoint, publication};

const OPERATION: &str = "resolve_activation_review_context";

pub(super) fn resolve(
    repository_root: &Path,
    activation_request_path: &Path,
    context_request_path: &Path,
) -> Result<ProspectiveResolveSuccess, ResolveFailure> {
    let request_target = if context_request_path.is_absolute() {
        context_request_path.to_owned()
    } else {
        repository_root.join(context_request_path)
    };
    let request_capture = publication::capture_file(
        repository_root,
        &request_target,
        operations::MAX_REQUEST_BYTES,
    )
    .map_err(|error| operations::request_capture_failure(error, context_request_path))?;
    let prospective = checkpoint::prepare_prospective_context(
        repository_root,
        activation_request_path,
        OPERATION,
    )
    .map_err(checkpoint_failure)?;
    let compiled = operations::compile_captured(
        repository_root,
        request_capture.bytes(),
        context_request_path,
        &prospective.authority,
    )?;
    let stored = storage::publish(repository_root, &compiled.artifacts, || {
        request_capture.revalidate().map_err(|error| {
            ResolveFailure::new(
                Some(prospective.authority.trusted_commit.clone()),
                "request_changed_during_resolution",
                error.to_string(),
                false,
                Vec::new(),
                vec![context_request_path.to_string_lossy().into_owned()],
                "retry after the ContextBuild request stops changing",
            )
            .into_activation_review()
        })?;
        compiled.final_revalidate(repository_root, &prospective.authority.trusted_commit)?;
        prospective
            .final_revalidate(repository_root)
            .map_err(checkpoint_failure)
    })
    .map_err(|failure| {
        failure
            .with_trusted_commit(&prospective.authority.trusted_commit)
            .into_activation_review()
    })?;
    Ok(ProspectiveResolveSuccess::new(ProspectiveSuccessInput {
        status: stored.status,
        trusted_commit: prospective.authority.trusted_commit.clone(),
        checkpoint_id: prospective.authority.checkpoint_id.clone(),
        checkpoint_hash: prospective.authority.checkpoint_hash.clone(),
        authority_basis_commit: prospective.authority.authority_basis_commit.clone(),
        activation_request_path: prospective.request_path().to_owned(),
        activation_request_hash: prospective.request_hash().to_owned(),
        predecessor_active_record_hash: prospective.predecessor_active_hash().map(str::to_owned),
        proposed_active_record_hash: prospective.proposed_active_record_hash().to_owned(),
        build_id: compiled.artifacts.build_id,
        context: stored.context,
        manifest: stored.manifest,
        affected_ids: compiled.artifacts.included_ids,
    }))
}

fn checkpoint_failure(failure: checkpoint::OperationFailure) -> ResolveFailure {
    let (trusted_commit, code, message, affected_ids) = failure.parts();
    ResolveFailure::new(
        trusted_commit,
        code,
        message,
        false,
        affected_ids,
        Vec::new(),
        "repair the exact activation proposal and retry",
    )
    .into_activation_review()
}
