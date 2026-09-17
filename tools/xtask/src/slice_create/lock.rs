use std::{
    collections::BTreeSet,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

use rustix::fs::{FileType, FlockOperation, Mode, OFlags, flock, fstat, open, openat};

use super::{request, worktree};
use crate::{git, slice_contract, slice_worktree};

pub(super) struct BootstrapLock {
    _file: File,
}

pub(super) fn verify_coordination_leases(
    contract: &slice_contract::SliceContract,
    registered: &[slice_worktree::Worktree],
    coordination: &Path,
    target_contract_path: &Path,
) -> Result<BTreeSet<PathBuf>, String> {
    match fs::symlink_metadata(coordination) {
        Ok(metadata) if metadata.file_type().is_dir() => {},
        Ok(_) => {
            return Err(format!(
                "coordination path {} must be a directory without symlinks",
                coordination.display()
            ));
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(BTreeSet::new());
        },
        Err(error) => {
            return Err(format!(
                "cannot inspect coordination path {}: {error}",
                coordination.display()
            ));
        },
    }
    let entries = match fs::read_dir(coordination) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => {
            return Err(format!(
                "cannot inspect coordination directory {}: {error}",
                coordination.display()
            ));
        },
    };
    let mut checked = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "cannot inspect coordination entry in {}: {error}",
                coordination.display()
            )
        })?;
        let entry_type = entry.file_type().map_err(|error| {
            format!(
                "cannot inspect coordination entry {}: {error}",
                entry.path().display()
            )
        })?;
        if entry_type.is_symlink() {
            return Err(format!(
                "coordination entry {} must not be a symlink",
                entry.path().display()
            ));
        }
        if !entry_type.is_dir() {
            continue;
        }
        let path = entry.path().join("slice-contract.json");
        if path == target_contract_path {
            continue;
        }
        let Some(bytes) = request::read_optional_contract(&path)? else {
            continue;
        };
        let active: slice_contract::SliceContract =
            serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid active Slice contract {}: {error}", path.display())
            })?;
        request::validate_slice_name(&active.slice)?;
        request::validate_base_ref_shape(&active.base_ref)?;
        let active_integration =
            worktree::unique_integration_worktree(registered, &active.base_ref).map_err(
                |error| format!("cannot verify active contract {}: {error}", path.display()),
            )?;
        slice_contract::validate_contract(&active_integration.path, &active).map_err(|error| {
            format!("invalid active Slice contract {}: {error}", path.display())
        })?;
        let canonical = fs::canonicalize(&path).map_err(|error| {
            format!("cannot resolve active contract {}: {error}", path.display())
        })?;
        checked.insert(canonical);
        slice_contract::ensure_lease_compatible(contract, &active)
            .map_err(|error| format!("{error}\nactive contract: {}", path.display()))?;
    }
    Ok(checked)
}

pub(super) fn acquire_bootstrap_lock(repository: &Path) -> Result<BootstrapLock, String> {
    let common = git::output_in(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        false,
    )?;
    let common = PathBuf::from(common.trim());
    let lock_path = common.join("yo-slice-create.lock");
    let directory = open(
        &common,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| {
        format!(
            "cannot open common Git directory {}: {error}",
            common.display()
        )
    })?;
    let fd = openat(
        &directory,
        "yo-slice-create.lock",
        OFlags::WRONLY | OFlags::CREATE | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| {
        format!(
            "cannot open Slice bootstrap lock {}: {error}",
            lock_path.display()
        )
    })?;
    let stat =
        fstat(&fd).map_err(|error| format!("cannot inspect Slice bootstrap lock: {error}"))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile || stat.st_nlink != 1 {
        return Err(format!(
            "Slice bootstrap lock {} must be a singly linked regular file",
            lock_path.display()
        ));
    }
    let file = File::from(fd);
    flock(&file, FlockOperation::NonBlockingLockExclusive).map_err(|error| {
        format!(
            "another cooperating Slice bootstrap is active at {}: {error}",
            lock_path.display()
        )
    })?;
    Ok(BootstrapLock { _file: file })
}
