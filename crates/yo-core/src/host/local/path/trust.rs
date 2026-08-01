use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

use super::super::{LocalWorkspaceHostIdentityError, io_error};

/// Validates the lexical path before canonicalization can hide an unsafe parent.
/// Returns the longest existing prefix, whose entry is protected from other users.
pub(super) fn validate_original_existing_prefix(
    path: &Path,
) -> Result<PathBuf, LocalWorkspaceHostIdentityError> {
    let effective_user = effective_user();
    let system_owner = system_owner()?;
    let mut components = path.components();
    if components.next() != Some(Component::RootDir) {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: path.to_owned(),
            reason: "the Workspace Host state root must begin at the filesystem root".to_owned(),
        });
    }

    let mut current = PathBuf::from("/");
    for component in components {
        let Component::Normal(component) = component else {
            return Err(LocalWorkspaceHostIdentityError::Invalid {
                path: path.to_owned(),
                reason: "the Workspace Host state root is not lexically normalized".to_owned(),
            });
        };
        let parent_metadata =
            fs::metadata(&current).map_err(|source| io_error("inspect", &current, source))?;
        validate_ancestor(&current, &parent_metadata, effective_user, system_owner)?;

        let next = current.join(component);
        let child_metadata = match fs::symlink_metadata(&next) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(current),
            Err(source) => return Err(io_error("inspect", &next, source)),
        };
        validate_protected_entry(
            &current,
            &parent_metadata,
            &next,
            &child_metadata,
            effective_user,
            system_owner,
        )?;
        current = next;
    }
    Ok(current)
}

pub(super) fn resolve_trusted_existing_path(
    path: &Path,
    reject_final_symlink: bool,
) -> Result<PathBuf, LocalWorkspaceHostIdentityError> {
    require_complete_original_path(path, reject_final_symlink)?;
    let first = fs::canonicalize(path).map_err(|source| io_error("resolve", path, source))?;
    validate_canonical_path_trust(&first)?;

    require_complete_original_path(path, reject_final_symlink)?;
    let second = fs::canonicalize(path).map_err(|source| io_error("resolve", path, source))?;
    if second != first {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: path.to_owned(),
            reason: "the Workspace Host state path resolved to different locations".to_owned(),
        });
    }
    Ok(first)
}

fn require_complete_original_path(
    path: &Path,
    reject_final_symlink: bool,
) -> Result<(), LocalWorkspaceHostIdentityError> {
    if validate_original_existing_prefix(path)? != path {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: path.to_owned(),
            reason: "the Workspace Host state path changed during validation".to_owned(),
        });
    }
    if reject_final_symlink {
        let metadata =
            fs::symlink_metadata(path).map_err(|source| io_error("inspect", path, source))?;
        if metadata.file_type().is_symlink() {
            return Err(LocalWorkspaceHostIdentityError::Invalid {
                path: path.to_owned(),
                reason: "the Workspace Host state root must not be a symbolic link".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_canonical_path_trust(root: &Path) -> Result<(), LocalWorkspaceHostIdentityError> {
    let effective_user = effective_user();
    let system_owner = system_owner()?;
    let mut child = root.to_owned();
    while let Some(parent) = child.parent() {
        let parent_metadata =
            fs::symlink_metadata(parent).map_err(|source| io_error("inspect", parent, source))?;
        validate_ancestor(parent, &parent_metadata, effective_user, system_owner)?;
        let child_metadata =
            fs::symlink_metadata(&child).map_err(|source| io_error("inspect", &child, source))?;
        validate_protected_entry(
            parent,
            &parent_metadata,
            &child,
            &child_metadata,
            effective_user,
            system_owner,
        )?;
        child = parent.to_owned();
    }
    Ok(())
}

fn validate_ancestor(
    path: &Path,
    metadata: &fs::Metadata,
    effective_user: u32,
    system_owner: u32,
) -> Result<(), LocalWorkspaceHostIdentityError> {
    if !metadata.is_dir() {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: path.to_owned(),
            reason: "a Workspace Host state ancestor is not a directory".to_owned(),
        });
    }
    if metadata.uid() != effective_user && metadata.uid() != system_owner {
        return Err(LocalWorkspaceHostIdentityError::Invalid {
            path: path.to_owned(),
            reason: "a Workspace Host state ancestor has an untrusted owner".to_owned(),
        });
    }
    Ok(())
}

fn validate_protected_entry(
    parent: &Path,
    parent_metadata: &fs::Metadata,
    child: &Path,
    child_metadata: &fs::Metadata,
    effective_user: u32,
    system_owner: u32,
) -> Result<(), LocalWorkspaceHostIdentityError> {
    if parent_metadata.permissions().mode() & 0o022 == 0 {
        return Ok(());
    }
    let trusted_parent_owner =
        parent_metadata.uid() == effective_user || parent_metadata.uid() == system_owner;
    let sticky_protection = parent_metadata.permissions().mode() & 0o1000 != 0
        && trusted_parent_owner
        && child_metadata.uid() == effective_user;
    if sticky_protection {
        return Ok(());
    }
    Err(LocalWorkspaceHostIdentityError::Invalid {
        path: parent.to_owned(),
        reason: format!(
            "a Workspace Host state ancestor is writable by other users and does not protect {}",
            child.display()
        ),
    })
}

fn effective_user() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn system_owner() -> Result<u32, LocalWorkspaceHostIdentityError> {
    fs::symlink_metadata(Path::new("/"))
        .map_err(|source| io_error("inspect", Path::new("/"), source))
        .map(|metadata| metadata.uid())
}
