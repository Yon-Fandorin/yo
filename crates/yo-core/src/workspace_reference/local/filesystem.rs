use std::{
    collections::{BTreeSet, HashSet},
    ffi::OsStr,
    path::{Path, PathBuf},
};

use super::{super::WorkspaceReferenceKind, git::ignored_paths};

pub(super) fn discover_entries(
    root: &Path,
    honor_git_ignore: bool,
) -> Result<(BTreeSet<(String, WorkspaceReferenceKind)>, bool), String> {
    let mut visible = BTreeSet::new();
    let mut skipped_non_utf8 = false;
    let mut frontier = vec![PathBuf::new()];
    while !frontier.is_empty() {
        let mut candidates = Vec::new();
        for relative in std::mem::take(&mut frontier) {
            let directory = root.join(&relative);
            let entries = std::fs::read_dir(&directory)
                .map_err(|error| format!("reading {} failed: {error}", directory.display()))?;
            for entry in entries {
                let entry =
                    entry.map_err(|error| format!("reading a workspace entry failed: {error}"))?;
                if entry.file_name() == OsStr::new(".git") {
                    continue;
                }
                let file_type = entry.file_type().map_err(|error| {
                    format!(
                        "reading the kind of {} failed: {error}",
                        entry.path().display()
                    )
                })?;
                if !file_type.is_symlink() && (file_type.is_dir() || file_type.is_file()) {
                    candidates.push((
                        relative.join(entry.file_name()),
                        if file_type.is_dir() {
                            WorkspaceReferenceKind::Directory
                        } else {
                            WorkspaceReferenceKind::File
                        },
                    ));
                }
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
                skipped_non_utf8 = true;
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
    }
    Ok((visible, skipped_non_utf8))
}
