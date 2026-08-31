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
    let shared = publication::lock_target_shared(repository_root, &directory)
        .map_err(|error| publication_failure(error, &directory))?;
    match shared
        .directory_state(&files)
        .map_err(|error| publication_failure(error, &directory))?
    {
        DirectoryState::Matches(verified) => {
            final_revalidate()?;
            let stored = stored_artifacts(repository_root, artifacts, &directory, "reused");
            drop(verified);
            return Ok(stored);
        },
        DirectoryState::Missing | DirectoryState::Different => {},
    }
    drop(shared);

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
    let stored = stored_artifacts(repository_root, artifacts, &directory, status);
    drop(verified);
    Ok(stored)
}

fn stored_artifacts(
    repository_root: &Path,
    artifacts: &BuildArtifacts,
    directory: &Path,
    status: &'static str,
) -> StoredArtifacts {
    StoredArtifacts {
        status,
        context: ArtifactReference {
            path: relative(repository_root, &directory.join("context.md")),
            hash: artifacts.context_hash.clone(),
            tokens: Some(artifacts.tokens),
        },
        manifest: ArtifactReference {
            path: relative(repository_root, &directory.join("manifest.json")),
            hash: artifacts.manifest_hash.clone(),
            tokens: None,
        },
    }
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

pub(super) fn build_directory(repository_root: &Path, build_id: &str) -> PathBuf {
    repository_root
        .join(".local-exclude/methexis/builds")
        .join(build_id.strip_prefix("sha256:").unwrap_or(build_id))
}

pub(super) fn publication_failure(error: PublicationError, path: &Path) -> ResolveFailure {
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
        PublicationError::DurabilityUnknown(error) => (
            "context_storage_recovery_required",
            format!("ContextBuild durability and rollback are uncertain: {error}"),
            false,
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

pub(super) fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, mpsc},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{BuildArtifacts, publish};

    fn artifacts() -> BuildArtifacts {
        BuildArtifacts {
            build_id: format!("sha256:{}", "a".repeat(64)),
            context: b"context\n".to_vec(),
            context_hash: format!("sha256:{}", "b".repeat(64)),
            manifest: b"manifest\n".to_vec(),
            manifest_hash: format!("sha256:{}", "c".repeat(64)),
            tokens: 1,
            included_ids: vec!["example.context".to_owned()],
        }
    }

    // 첫 reuse가 final revalidation 중이어도 같은 immutable build를 읽는 두 번째 reuse는
    // 배타 lock 충돌 없이 끝나야 실제 여러 reviewer delivery가 겹쳐 실행될 수 있습니다.
    #[test]
    fn concurrent_existing_build_reuse_does_not_serialize_readers() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = Arc::new(std::env::temp_dir().join(format!(
            "methexis-context-reuse-{}-{unique}",
            std::process::id()
        )));
        fs::create_dir(&*root).unwrap();
        assert_eq!(
            publish(&root, &artifacts(), || Ok(())).unwrap().status,
            "created"
        );

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_root = Arc::clone(&root);
        let first = thread::spawn(move || {
            publish(&first_root, &artifacts(), || {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            })
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        let second = publish(&root, &artifacts(), || Ok(()));
        release_tx.send(()).unwrap();
        let first = first.join().unwrap();

        assert_eq!(first.unwrap().status, "reused");
        assert_eq!(second.unwrap().status, "reused");
        fs::remove_dir_all(&*root).unwrap();
    }
}
