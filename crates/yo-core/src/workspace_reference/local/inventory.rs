use std::path::Path;

use super::{
    super::{
        WorkspaceReference, WorkspaceReferenceCandidate, WorkspaceReferenceKind,
        WorkspaceReferenceSearchStatus,
    },
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
    let honor_git_ignore = is_git_workspace(root)?;
    let (mut paths, mut incomplete) = discover_entries(root, honor_git_ignore)?;
    if honor_git_ignore {
        let (tracked, tracked_incomplete) = discover_tracked_entries(root)?;
        paths.extend(tracked);
        incomplete |= tracked_incomplete;
    }
    let root_identity = format!(
        "local-root:{}",
        hex_bytes(root.as_os_str().as_encoded_bytes())
    );
    let execution_environment_identity = format!("local-host:{workspace_host_id}");
    let workspace_identity = format!("{workspace_host_id}:{root_identity}");
    let entries = paths
        .into_iter()
        .map(|(path, kind)| {
            let kind_name = match kind {
                WorkspaceReferenceKind::File => "file",
                WorkspaceReferenceKind::Directory => "directory",
            };
            WorkspaceReference::new(
                format!("local:{kind_name}:{}", hex_bytes(path.as_bytes())),
                execution_environment_identity.clone(),
                workspace_identity.clone(),
                root_identity.clone(),
                path,
                kind,
            )
            .map(WorkspaceReferenceCandidate::new)
            .map_err(|error| format!("normalizing a discovered workspace path failed: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Inventory {
        entries,
        status: if incomplete {
            WorkspaceReferenceSearchStatus::Incomplete(
                "Some non-UTF-8 or unreadable workspace paths were skipped".to_owned(),
            )
        } else {
            WorkspaceReferenceSearchStatus::Complete
        },
    })
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
