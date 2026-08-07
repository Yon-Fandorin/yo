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

const MAX_RETAINED_PATHS: usize = 64;
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
                "Slice coordination directory {} exceeds the {MAX_RETAINED_PATHS}-entry reporting limit after excluding the planned standard contract",
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
