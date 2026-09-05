use std::{
    ffi::OsString,
    os::{
        fd::OwnedFd,
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Path, PathBuf},
};

use rustix::{
    fs::{Dir, Mode, OFlags, open, openat},
    io::Errno,
};

use crate::bounded_file;

const MAX_RETAINED_PATHS: usize = 64;
const MAX_CLEANUP_PATHS: usize = 256;
const OPEN_DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(super) fn retained_paths(
    workspace: &Path,
    slice: &str,
    remove_standard_contract: bool,
) -> Result<Vec<PathBuf>, String> {
    let coordination = workspace
        .join(".local-exclude")
        .join("coordination")
        .join(slice);
    let workspace_fd = open(workspace, OPEN_DIRECTORY, Mode::empty()).map_err(|error| {
        format!(
            "cannot open workspace root {} while inspecting retained coordination: {error}",
            workspace.display()
        )
    })?;
    let Some(local) = open_optional_directory(
        &workspace_fd,
        ".local-exclude",
        &workspace.join(".local-exclude"),
    )?
    else {
        return Ok(Vec::new());
    };
    let Some(coordination_root) = open_optional_directory(
        &local,
        "coordination",
        &workspace.join(".local-exclude/coordination"),
    )?
    else {
        return Ok(Vec::new());
    };
    let Some(slice_directory) = open_optional_directory(&coordination_root, slice, &coordination)?
    else {
        return Ok(Vec::new());
    };

    let entries = Dir::read_from(&slice_directory).map_err(|error| {
        format!(
            "cannot enumerate Slice coordination directory {}: {error}",
            coordination.display()
        )
    })?;
    let mut retained = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "cannot enumerate Slice coordination directory {}: {error}",
                coordination.display()
            )
        })?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        if remove_standard_contract && name == b"slice-contract.json" {
            continue;
        }
        if retained.len() == MAX_RETAINED_PATHS {
            return Err(format!(
                "Slice coordination directory {} exceeds the {MAX_RETAINED_PATHS}-entry reporting limit",
                coordination.display()
            ));
        }
        retained.push(coordination.join(OsString::from_vec(name.to_vec())));
    }
    retained.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });
    Ok(retained)
}

pub(super) fn remove_directory(
    workspace: &Path,
    slice: &str,
    expected_paths: &[PathBuf],
) -> Result<(), String> {
    let coordination = workspace
        .join(".local-exclude")
        .join("coordination")
        .join(slice);
    let current = cleanup_paths(workspace, slice)?;
    if current != expected_paths {
        return Err("Slice coordination cleanup paths changed after planning".to_owned());
    }
    let metadata = std::fs::symlink_metadata(&coordination).map_err(|error| {
        format!(
            "cannot inspect Slice coordination directory {} before cleanup: {error}",
            coordination.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Slice coordination cleanup target must be a real directory".to_owned());
    }
    std::fs::remove_dir_all(&coordination).map_err(|error| {
        format!(
            "cannot remove completed Slice coordination directory {}: {error}",
            coordination.display()
        )
    })?;
    let parent = bounded_file::open_directory(
        coordination
            .parent()
            .expect("standard Slice coordination has a parent"),
        "Slice coordination cleanup",
    )?;
    bounded_file::sync_directory(&parent, "Slice coordination cleanup")
}

pub(super) fn cleanup_paths(workspace: &Path, slice: &str) -> Result<Vec<PathBuf>, String> {
    let coordination = workspace
        .join(".local-exclude")
        .join("coordination")
        .join(slice);
    let metadata = match std::fs::symlink_metadata(&coordination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "cannot inspect Slice coordination cleanup root {}: {error}",
                coordination.display()
            ));
        },
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Slice coordination cleanup root must be a real directory".to_owned());
    }
    let mut paths = Vec::new();
    collect_cleanup_paths(&coordination, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_cleanup_paths(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "cannot enumerate Slice coordination cleanup directory {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "cannot read Slice coordination cleanup directory {}: {error}",
                directory.display()
            )
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if paths.len() == MAX_CLEANUP_PATHS {
            return Err(format!(
                "Slice coordination cleanup exceeds the {MAX_CLEANUP_PATHS}-path reporting limit"
            ));
        }
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "cannot inspect Slice coordination cleanup path {}: {error}",
                path.display()
            )
        })?;
        paths.push(path.clone());
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            collect_cleanup_paths(&path, paths)?;
        }
    }
    Ok(())
}

fn open_optional_directory(
    parent: &OwnedFd,
    name: &str,
    path: &Path,
) -> Result<Option<OwnedFd>, String> {
    match openat(parent, name, OPEN_DIRECTORY, Mode::empty()) {
        Ok(directory) => Ok(Some(directory)),
        Err(Errno::NOENT) => Ok(None),
        Err(error) => Err(format!(
            "cannot open Slice coordination directory {} without following symlinks: {error}",
            path.display()
        )),
    }
}
