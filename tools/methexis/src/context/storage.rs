//! Immutable, create-if-absent ContextBuild publication and verified reuse.

use std::path::{Path, PathBuf};

use super::{
    payload::BuildArtifacts,
    wire::{ArtifactReference, ResolveFailure},
};
use crate::publication::{self, DirectoryState, GuardedDirectoryError, PublicationError};

pub(super) struct StoredArtifacts {
    pub(super) status: &'static str,
    pub(super) context: ArtifactReference,
    pub(super) manifest: ArtifactReference,
}

pub(super) fn publish(
    repository_root: &Path,
    artifacts: &BuildArtifacts,
    mut final_revalidate: impl FnMut() -> Result<(), ResolveFailure>,
) -> Result<StoredArtifacts, ResolveFailure> {
    let directory = build_directory(repository_root, &artifacts.build_id);
    let files = [
        ("context.md", artifacts.context.as_slice()),
        ("manifest.json", artifacts.manifest.as_slice()),
    ];
    let lock = publication::lock_target(repository_root, &directory)
        .map_err(|error| publication_failure(error, &directory))?;
    let (status, verified) = match lock
        .directory_state(&files)
        .map_err(|error| publication_failure(error, &directory))?
    {
        DirectoryState::Matches(verified) => {
            final_revalidate()?;
            ("reused", verified)
        },
        DirectoryState::Different => {
            quarantine(repository_root, artifacts)?;
            return Err(ResolveFailure::new(
                None,
                "context_build_collision",
                "existing BuildId directory differs from the deterministic artifacts",
                false,
                Vec::new(),
                vec![relative(repository_root, &directory)],
                "inspect the existing build and quarantined candidate for corruption",
            ));
        },
        DirectoryState::Missing => {
            match lock.atomic_create_directory_guarded(&files, &mut final_revalidate) {
                Ok(()) => {},
                Err(GuardedDirectoryError::Guard(failure)) => return Err(failure),
                Err(GuardedDirectoryError::Publication(error)) => {
                    return Err(publication_failure(error, &directory));
                },
            }
            match lock
                .directory_state(&files)
                .map_err(|error| publication_failure(error, &directory))?
            {
                DirectoryState::Matches(verified) => ("created", verified),
                DirectoryState::Missing | DirectoryState::Different => {
                    return Err(ResolveFailure::new(
                        None,
                        "context_publication_incomplete",
                        "published ContextBuild did not verify as the exact artifact set",
                        false,
                        Vec::new(),
                        vec![relative(repository_root, &directory)],
                        "inspect local storage and retry",
                    ));
                },
            }
        },
    };
    let context_path = directory.join("context.md");
    let manifest_path = directory.join("manifest.json");
    let stored = StoredArtifacts {
        status,
        context: ArtifactReference {
            path: relative(repository_root, &context_path),
            hash: artifacts.context_hash.clone(),
            tokens: Some(artifacts.tokens),
        },
        manifest: ArtifactReference {
            path: relative(repository_root, &manifest_path),
            hash: artifacts.manifest_hash.clone(),
            tokens: None,
        },
    };
    drop(verified);
    Ok(stored)
}

fn quarantine(repository_root: &Path, artifacts: &BuildArtifacts) -> Result<(), ResolveFailure> {
    let build = artifacts
        .build_id
        .strip_prefix("sha256:")
        .unwrap_or(&artifacts.build_id);
    let manifest = artifacts
        .manifest_hash
        .strip_prefix("sha256:")
        .unwrap_or(&artifacts.manifest_hash);
    let target = repository_root
        .join(".local-exclude/methexis/quarantine")
        .join(format!("{build}-{manifest}"));
    let files = [
        ("context.md", artifacts.context.as_slice()),
        ("manifest.json", artifacts.manifest.as_slice()),
    ];
    let lock = publication::lock_target(repository_root, &target)
        .map_err(|error| publication_failure(error, &target))?;
    match lock
        .directory_state(&files)
        .map_err(|error| publication_failure(error, &target))?
    {
        DirectoryState::Matches(_) => Ok(()),
        DirectoryState::Missing => lock
            .atomic_create_directory(&files)
            .map_err(|error| publication_failure(error, &target)),
        DirectoryState::Different => Err(ResolveFailure::new(
            None,
            "context_quarantine_collision",
            "quarantine destination contains different bytes",
            false,
            Vec::new(),
            vec![relative(repository_root, &target)],
            "inspect local ContextBuild quarantine storage",
        )),
    }
}

fn build_directory(repository_root: &Path, build_id: &str) -> PathBuf {
    repository_root
        .join(".local-exclude/methexis/builds")
        .join(build_id.strip_prefix("sha256:").unwrap_or(build_id))
}

fn publication_failure(error: PublicationError, path: &Path) -> ResolveFailure {
    let (code, message, retryable) = match error {
        PublicationError::OutsideRepository => (
            "context_path_invalid",
            "ContextBuild path escapes the repository".to_owned(),
            false,
        ),
        PublicationError::Symlink(path) => (
            "context_path_symlink",
            format!("ContextBuild path uses symlink `{}`", path.display()),
            false,
        ),
        PublicationError::NotDirectory(path) => (
            "context_path_not_directory",
            format!(
                "ContextBuild parent is not a directory `{}`",
                path.display()
            ),
            false,
        ),
        PublicationError::Locked(error) => (
            "context_build_locked",
            format!("ContextBuild target is locked: {error}"),
            true,
        ),
        PublicationError::Io(error) => ("context_storage_failed", error.to_string(), false),
    };
    ResolveFailure::new(
        None,
        code,
        message,
        retryable,
        Vec::new(),
        vec![path.to_string_lossy().into_owned()],
        "inspect local ContextBuild storage and retry",
    )
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
