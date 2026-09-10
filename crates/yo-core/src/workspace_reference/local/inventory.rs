use std::path::Path;

use rustix::{fd::OwnedFd, fs::fstat};

use super::{
    super::{
        WorkspaceReference, WorkspaceReferenceCandidate, WorkspaceReferenceKind,
        WorkspaceReferenceSearchStatus,
    },
    DiscoveryBudget,
    filesystem::discover_entries,
    git::{discover_tracked_entries, is_git_workspace},
};
use crate::WorkspaceHostId;

pub(super) struct Inventory {
    pub(super) entries: Vec<WorkspaceReferenceCandidate>,
    pub(super) status: WorkspaceReferenceSearchStatus,
}

pub(super) fn build_inventory(
    root: &Path,
    workspace_host_id: WorkspaceHostId,
) -> Result<Inventory, String> {
    let descriptor = super::pin_root(root)
        .map_err(|error| format!("workspace root {} is unavailable: {error}", root.display()))?;
    let root_identity = root_identity(root, &descriptor)?;
    let honor_git_ignore = is_git_workspace(root)?;
    let mut budget = DiscoveryBudget::default();
    let (mut paths, mut incomplete) =
        discover_entries(root, &descriptor, honor_git_ignore, &mut budget)?;
    if honor_git_ignore && !budget.exhausted {
        let (tracked, tracked_incomplete) =
            discover_tracked_entries(root, &descriptor, &mut budget)?;
        paths.extend(tracked);
        incomplete |= tracked_incomplete;
    }
    let current_root = super::pin_root(root).map_err(|error| error.to_string())?;
    if root_identity != self::root_identity(root, &current_root)? {
        return Err("workspace root changed during discovery".to_owned());
    }
    let entries = paths
        .into_iter()
        .map(|(path, kind)| {
            reference(&root_identity, workspace_host_id, path, kind)
                .map(WorkspaceReferenceCandidate::new)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Inventory {
        entries,
        status: if incomplete {
            WorkspaceReferenceSearchStatus::Incomplete(
                "Workspace discovery was bounded or some paths were unavailable or non-UTF-8"
                    .to_owned(),
            )
        } else {
            WorkspaceReferenceSearchStatus::Complete
        },
    })
}

pub(super) fn root_identity(root: &Path, descriptor: &OwnedFd) -> Result<String, String> {
    let stat = fstat(descriptor).map_err(|error| error.to_string())?;
    Ok(format!(
        "local-root:{}:{:x}:{:x}",
        hex_bytes(root.as_os_str().as_encoded_bytes()),
        stat.st_dev,
        stat.st_ino
    ))
}

pub(super) fn reference(
    root_identity: &str,
    host: WorkspaceHostId,
    path: String,
    kind: WorkspaceReferenceKind,
) -> Result<WorkspaceReference, String> {
    let kind_name = match kind {
        WorkspaceReferenceKind::File => "file",
        WorkspaceReferenceKind::Directory => "directory",
    };
    WorkspaceReference::new(
        format!("local:{kind_name}:{}", hex_bytes(path.as_bytes())),
        format!("local-host:{host}"),
        format!("{host}:{root_identity}"),
        root_identity,
        path,
        kind,
    )
    .map_err(|error| format!("normalizing a discovered workspace path failed: {error}"))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
