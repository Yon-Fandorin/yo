use std::{
    collections::{BTreeSet, HashSet},
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use rustix::{
    fd::OwnedFd,
    fs::{AtFlags, Dir, FileType, statat},
};

use super::{super::WorkspaceReferenceKind, DiscoveryBudget, git::ignored_paths, pin_directory};

pub(super) fn discover_entries(
    root: &Path,
    root_descriptor: &OwnedFd,
    honor_git_ignore: bool,
    budget: &mut DiscoveryBudget,
) -> Result<(BTreeSet<(String, WorkspaceReferenceKind)>, bool), String> {
    let mut visible = BTreeSet::new();
    let mut incomplete = false;
    let mut frontier = vec![PathBuf::new()];
    while !frontier.is_empty() {
        let mut candidates = Vec::new();
        for relative in std::mem::take(&mut frontier) {
            let directory = root.join(&relative);
            let descriptor = pin_directory(root_descriptor, &relative).map_err(|error| {
                format!(
                    "opening {} without symlinks failed: {error}",
                    directory.display()
                )
            })?;
            let entries = Dir::read_from(&descriptor)
                .map_err(|error| format!("reading {} failed: {error}", directory.display()))?;
            for entry in entries {
                let entry =
                    entry.map_err(|error| format!("reading a workspace entry failed: {error}"))?;
                let name = entry.file_name().to_bytes();
                if matches!(name, b"." | b".." | b".git") {
                    continue;
                }
                let path = relative.join(OsStr::from_bytes(name));
                if !budget.admit(&path) {
                    break;
                }
                let file_type = if entry.file_type() == FileType::Unknown {
                    let metadata =
                        statat(&descriptor, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW).map_err(
                            |error| {
                                format!("reading the kind of {} failed: {error}", path.display())
                            },
                        )?;
                    FileType::from_raw_mode(metadata.st_mode)
                } else {
                    entry.file_type()
                };
                let kind = match file_type {
                    FileType::Directory => WorkspaceReferenceKind::Directory,
                    FileType::RegularFile => WorkspaceReferenceKind::File,
                    _ => continue,
                };
                candidates.push((path, kind));
            }
            if budget.exhausted {
                break;
            }
        }
        let raw_paths = candidates
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        let ignored = if honor_git_ignore {
            ignored_paths(root, &raw_paths)?
        } else {
            HashSet::new()
        };
        for (candidate, kind) in candidates {
            let Some(path) = candidate.to_str() else {
                incomplete = true;
                continue;
            };
            let normalized = path.replace(std::path::MAIN_SEPARATOR, "/");
            if !ignored.contains(&normalized) {
                visible.insert((normalized, kind));
                if kind == WorkspaceReferenceKind::Directory {
                    frontier.push(candidate);
                }
            }
        }
        if budget.exhausted {
            incomplete = true;
            break;
        }
    }
    Ok((visible, incomplete))
}
