//! Independent, non-publishing ContextBuild reproduction and deep verification.

use std::path::Path;

use super::{
    hash, operations,
    payload::BuildArtifacts,
    storage,
    wire::{ArtifactReference, ResolveFailure, VerifySuccess},
};
use crate::{
    checkpoint,
    publication::{self, DirectoryState},
};

pub(super) fn run(
    repository_root: &Path,
    request_path: &Path,
    expected_build_id: &str,
) -> Result<VerifySuccess, ResolveFailure> {
    run_with_before_final(repository_root, request_path, expected_build_id, || {})
}

fn run_with_before_final(
    repository_root: &Path,
    request_path: &Path,
    expected_build_id: &str,
    before_final: impl FnOnce(),
) -> Result<VerifySuccess, ResolveFailure> {
    if !hash::valid(expected_build_id) {
        return Err(verification_failure(
            None,
            "invalid_context_build_id",
            "expected BuildId must use canonical sha256:<64 lowercase hex> syntax",
            Vec::new(),
        ));
    }
    let request_target = if request_path.is_absolute() {
        request_path.to_owned()
    } else {
        repository_root.join(request_path)
    };
    let request_capture = publication::capture_file(
        repository_root,
        &request_target,
        operations::MAX_REQUEST_BYTES,
    )
    .map_err(|error| {
        operations::request_capture_failure(error, request_path).into_verification()
    })?;
    let authority = checkpoint::resolve_context_authority(repository_root)
        .map_err(operations::authority_failure)
        .map_err(ResolveFailure::into_verification)?;
    let compiled = operations::compile_captured(
        repository_root,
        request_capture.bytes(),
        request_path,
        &authority,
    )
    .map_err(ResolveFailure::into_verification)?;
    if compiled.artifacts.build_id != expected_build_id {
        return Err(verification_failure(
            Some(authority.trusted_commit),
            "context_build_identity_mismatch",
            format!(
                "independent compilation derived `{}` instead of expected `{expected_build_id}`",
                compiled.artifacts.build_id
            ),
            Vec::new(),
        ));
    }

    verify_artifacts(
        repository_root,
        expected_build_id,
        &authority.trusted_commit,
        &compiled.artifacts,
        before_final,
        || {
            request_capture.revalidate().map_err(|error| {
                verification_failure(
                    Some(authority.trusted_commit.clone()),
                    "request_changed_during_verification",
                    error.to_string(),
                    vec![request_path.to_string_lossy().into_owned()],
                )
            })?;
            compiled
                .final_revalidate(repository_root, &authority.trusted_commit)
                .map_err(ResolveFailure::into_verification)?;
            checkpoint::final_revalidate_context_authority(repository_root, &authority)
                .map_err(operations::authority_failure)
                .map_err(ResolveFailure::into_verification)
        },
    )
}

fn verify_artifacts(
    repository_root: &Path,
    expected_build_id: &str,
    trusted_commit: &str,
    artifacts: &BuildArtifacts,
    before_final: impl FnOnce(),
    final_revalidate: impl FnOnce() -> Result<(), ResolveFailure>,
) -> Result<VerifySuccess, ResolveFailure> {
    let directory = storage::build_directory(repository_root, expected_build_id);
    let files = [
        ("context.md", artifacts.context.as_slice()),
        ("manifest.json", artifacts.manifest.as_slice()),
    ];
    let lock = publication::lock_target(repository_root, &directory)
        .map_err(|error| storage::publication_failure(error, &directory).into_verification())?;
    let verified = match lock
        .directory_state(&files)
        .map_err(|error| storage::publication_failure(error, &directory).into_verification())?
    {
        DirectoryState::Matches(verified) => verified,
        DirectoryState::Missing => {
            return Err(verification_failure(
                Some(trusted_commit.to_owned()),
                "context_build_missing",
                "the expected managed ContextBuild does not exist",
                vec![storage::relative(repository_root, &directory)],
            ));
        },
        DirectoryState::Different => {
            return Err(build_mismatch(repository_root, &directory, trusted_commit));
        },
    };

    before_final();
    final_revalidate().map_err(ResolveFailure::into_verification)?;
    let unchanged = lock
        .revalidate_directory(&verified, &files)
        .map_err(|error| storage::publication_failure(error, &directory).into_verification())?;
    if !unchanged {
        return Err(build_mismatch(repository_root, &directory, trusted_commit));
    }

    Ok(VerifySuccess::new(
        trusted_commit.to_owned(),
        expected_build_id.to_owned(),
        ArtifactReference {
            path: storage::relative(repository_root, &directory.join("context.md")),
            hash: artifacts.context_hash.clone(),
            tokens: Some(artifacts.tokens),
        },
        ArtifactReference {
            path: storage::relative(repository_root, &directory.join("manifest.json")),
            hash: artifacts.manifest_hash.clone(),
            tokens: None,
        },
    ))
}

fn build_mismatch(
    repository_root: &Path,
    directory: &Path,
    trusted_commit: &str,
) -> ResolveFailure {
    verification_failure(
        Some(trusted_commit.to_owned()),
        "context_build_verification_failed",
        "managed ContextBuild identity, closed file set, path types, or bytes differ",
        vec![storage::relative(repository_root, directory)],
    )
}

fn verification_failure(
    trusted_commit: Option<String>,
    code: &str,
    message: impl Into<String>,
    paths: Vec<String>,
) -> ResolveFailure {
    ResolveFailure::new(
        trusted_commit,
        code,
        message,
        false,
        Vec::new(),
        paths,
        "repair the request, authority, or managed ContextBuild and retry verification",
    )
    .into_verification()
}

#[cfg(test)]
#[path = "verify/tests.rs"]
mod tests;
