use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    thread,
    time::Duration,
};

use trust::{resolve_trusted_existing_path, validate_original_existing_prefix};

use super::{LocalWorkspaceHostIdentityError, io_error, sync_directory};

mod trust;

const DIRECTORY_MODE: u32 = 0o700;
const CREATION_SETTLE_ATTEMPTS: usize = 100;
const CREATION_SETTLE_DELAY: Duration = Duration::from_millis(1);

pub(super) fn prepare_state_root(root: &Path) -> Result<PathBuf, LocalWorkspaceHostIdentityError> {
    if !root.is_absolute() {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: root.to_owned(),
            reason: "the Workspace Host state root must be an absolute path".to_owned(),
        });
    }

    let existing_ancestor = validate_original_existing_prefix(root)?;
    if existing_ancestor != root {
        create_directory_path(root, &existing_ancestor)?;
    }
    let resolved_root = resolve_trusted_existing_path(root, true)?;
    let metadata =
        fs::symlink_metadata(&resolved_root).map_err(|source| io_error("inspect", root, source))?;
    if !metadata.is_dir() {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: root.to_owned(),
            reason: "the Workspace Host state root is not a directory".to_owned(),
        });
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode != DIRECTORY_MODE {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: root.to_owned(),
            reason: format!("Workspace Host state root permissions {mode:o} are not 700"),
        });
    }
    if metadata.uid() != effective_user() {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: root.to_owned(),
            reason: "the Workspace Host state root is not owned by the current user".to_owned(),
        });
    }
    sync_directory_entry(&resolved_root)?;
    Ok(resolved_root)
}

fn create_directory_path(
    root: &Path,
    existing_ancestor: &Path,
) -> Result<PathBuf, LocalWorkspaceHostIdentityError> {
    let canonical_ancestor = resolve_trusted_existing_path(existing_ancestor, false)?;
    settle_existing_ancestor(&canonical_ancestor)?;
    let suffix = root.strip_prefix(existing_ancestor).map_err(|_| {
        LocalWorkspaceHostIdentityError::Invalid {
            path: root.to_owned(),
            reason: "the Workspace Host state root escaped its existing ancestor".to_owned(),
        }
    })?;
    let mut current = canonical_ancestor;
    for component in suffix.components() {
        let Component::Normal(component) = component else {
            return Err(LocalWorkspaceHostIdentityError::Invalid {
                path: root.to_owned(),
                reason: "the Workspace Host state root is not lexically normalized".to_owned(),
            });
        };
        current.push(component);
        let mut builder = fs::DirBuilder::new();
        builder.mode(DIRECTORY_MODE);
        match builder.create(&current) {
            Ok(()) => fs::set_permissions(&current, fs::Permissions::from_mode(DIRECTORY_MODE))
                .map_err(|source| io_error("set permissions on", &current, source))?,
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                wait_for_created_directory(&current)?;
            },
            Err(source) => return Err(io_error("create", &current, source)),
        }
        sync_directory_entry(&current)?;
    }
    Ok(current)
}

fn settle_existing_ancestor(path: &Path) -> Result<(), LocalWorkspaceHostIdentityError> {
    for attempt in 0..CREATION_SETTLE_ATTEMPTS {
        let metadata =
            fs::symlink_metadata(path).map_err(|source| io_error("inspect", path, source))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(LocalWorkspaceHostIdentityError::Invalid {
                path: path.to_owned(),
                reason: "a Workspace Host state ancestor is not a directory".to_owned(),
            });
        }
        let mode = metadata.permissions().mode() & 0o777;
        let settling = is_restricted_creation_mode(&metadata, mode);
        if settling && attempt + 1 < CREATION_SETTLE_ATTEMPTS {
            thread::sleep(CREATION_SETTLE_DELAY);
            continue;
        }
        if settling {
            return Err(LocalWorkspaceHostIdentityError::Invalid {
                path: path.to_owned(),
                reason: format!(
                    "Workspace Host state ancestor permissions {mode:o} did not settle to 700"
                ),
            });
        }
        return sync_directory_entry(path);
    }
    unreachable!("the bounded ancestor-settle loop always returns")
}

fn wait_for_created_directory(path: &Path) -> Result<(), LocalWorkspaceHostIdentityError> {
    for attempt in 0..CREATION_SETTLE_ATTEMPTS {
        let metadata =
            fs::symlink_metadata(path).map_err(|source| io_error("inspect", path, source))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(LocalWorkspaceHostIdentityError::Invalid {
                path: path.to_owned(),
                reason: "a Workspace Host state path component is not a directory".to_owned(),
            });
        }
        let mode = metadata.permissions().mode() & 0o777;
        if mode == DIRECTORY_MODE && metadata.uid() == effective_user() {
            return Ok(());
        }
        if attempt + 1 < CREATION_SETTLE_ATTEMPTS && is_restricted_creation_mode(&metadata, mode) {
            thread::sleep(CREATION_SETTLE_DELAY);
            continue;
        }
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: path.to_owned(),
            reason: format!(
                "Workspace Host state path component permissions {mode:o} or owner are not 700/current-user"
            ),
        });
    }
    unreachable!("the bounded creation-settle loop always returns")
}

fn is_restricted_creation_mode(metadata: &fs::Metadata, mode: u32) -> bool {
    metadata.uid() == effective_user() && mode != DIRECTORY_MODE && mode & !DIRECTORY_MODE == 0
}

fn effective_user() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn sync_directory_entry(path: &Path) -> Result<(), LocalWorkspaceHostIdentityError> {
    sync_directory(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}
