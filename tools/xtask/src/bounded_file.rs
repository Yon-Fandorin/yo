use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Write},
    os::fd::OwnedFd,
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
};

use rustix::{
    fs::{
        AtFlags, FileType, Mode, OFlags, RenameFlags, fstat, mkdirat, open, openat, renameat_with,
        unlinkat,
    },
    io::Errno,
};
use sha2::{Digest, Sha256};

const READ_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NONBLOCK)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const CREATE_FLAGS: OFlags = OFlags::WRONLY
    .union(OFlags::CREATE)
    .union(OFlags::EXCL)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(1);

pub(crate) fn read_regular(path: &Path, limit: usize, label: &str) -> Result<Vec<u8>, String> {
    let parent_path = path
        .parent()
        .ok_or_else(|| format!("{label} path {} has no parent", path.display()))?;
    let target = path
        .file_name()
        .ok_or_else(|| format!("{label} path {} has no file name", path.display()))?;
    let parent = open_directory(parent_path, label)?;
    read_regular_at(&parent, target, path, limit, label)?
        .ok_or_else(|| format!("cannot open {label} {}: {}", path.display(), Errno::NOENT))
}

pub(crate) fn read_regular_at(
    parent: &OwnedFd,
    name: &std::ffi::OsStr,
    display_path: &Path,
    limit: usize,
    label: &str,
) -> Result<Option<Vec<u8>>, String> {
    let fd = match openat(parent, name, READ_FLAGS, Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot open {label} {}: {error}",
                display_path.display()
            ));
        },
    };
    let stat = fstat(&fd)
        .map_err(|error| format!("cannot inspect {label} {}: {error}", display_path.display()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile || stat.st_nlink != 1 {
        return Err(format!("{label} must be a singly linked regular file"));
    }
    let declared =
        usize::try_from(stat.st_size).map_err(|_| format!("{label} has an unsupported size"))?;
    if declared > limit {
        return Err(format!("{label} exceeds the {limit}-byte limit"));
    }
    let mut bytes = Vec::with_capacity(declared.min(limit));
    File::from(fd)
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {label} {}: {error}", display_path.display()))?;
    if bytes.len() > limit {
        return Err(format!("{label} exceeds the {limit}-byte limit"));
    }
    Ok(Some(bytes))
}

pub(crate) fn publish_new_or_exact(
    path: &Path,
    expected: &[u8],
    limit: usize,
    label: &str,
) -> Result<bool, String> {
    publish_new_or_exact_with(path, expected, limit, label, |file, bytes| {
        file.write_all(bytes).map_err(|error| {
            format!("cannot write prepared {label} {}: {error}", path.display())
        })?;
        file.sync_all()
            .map_err(|error| format!("cannot sync prepared {label} {}: {error}", path.display()))
    })
}

pub(crate) fn remove_regular_matching_sha256(
    path: &Path,
    expected_hash: &str,
    limit: usize,
    label: &str,
) -> Result<bool, String> {
    remove_regular_matching_sha256_with_hooks(
        path,
        expected_hash,
        limit,
        label,
        || Ok(()),
        |parent| sync_directory(parent, label),
    )
}

fn remove_regular_matching_sha256_with_hooks(
    path: &Path,
    expected_hash: &str,
    limit: usize,
    label: &str,
    mut before_claim: impl FnMut() -> Result<(), String>,
    mut sync_parent: impl FnMut(&OwnedFd) -> Result<(), String>,
) -> Result<bool, String> {
    let parent_path = path
        .parent()
        .ok_or_else(|| format!("{label} path {} has no parent", path.display()))?;
    let target = path
        .file_name()
        .ok_or_else(|| format!("{label} path {} has no file name", path.display()))?;
    let hash_suffix = expected_hash
        .strip_prefix("sha256:")
        .filter(|suffix| {
            suffix.len() == 64
                && suffix
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
        .ok_or_else(|| format!("{label} expected hash is not canonical SHA-256"))?;
    let mut claimed = OsString::from(".");
    claimed.push(target);
    claimed.push(format!(".yo-remove-{hash_suffix}"));
    let parent = open_directory(parent_path, label)?;
    let claimed_path = parent_path.join(&claimed);
    let mut removed = false;

    loop {
        let claimed_bytes = match read_regular_at(&parent, &claimed, &claimed_path, limit, label) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(restore_claimed(
                    &parent,
                    &claimed,
                    target,
                    &claimed_path,
                    path,
                    label,
                    &mut sync_parent,
                    error,
                ));
            },
        };
        if let Some(bytes) = claimed_bytes {
            // A previous attempt may have stopped after the atomic claim. Establish that
            // directory state before deleting the claimed inode.
            sync_parent(&parent)?;
            if let Err(error) = exact_sha256(path, expected_hash, &bytes, label) {
                return Err(restore_claimed(
                    &parent,
                    &claimed,
                    target,
                    &claimed_path,
                    path,
                    label,
                    &mut sync_parent,
                    error,
                ));
            }
            unlinkat(&parent, &claimed, AtFlags::empty()).map_err(|error| {
                format!(
                    "cannot remove claimed {label} {}: {error}",
                    claimed_path.display()
                )
            })?;
            sync_parent(&parent)?;
            removed = true;
            continue;
        }

        let Some(bytes) = read_regular_at(&parent, target, path, limit, label)? else {
            // This also makes a preceding unlink durable when its parent sync failed.
            sync_parent(&parent)?;
            return Ok(removed);
        };
        if let Err(error) = exact_sha256(path, expected_hash, &bytes, label) {
            sync_parent(&parent)?;
            return Err(error);
        }

        before_claim()?;
        match renameat_with(&parent, target, &parent, &claimed, RenameFlags::NOREPLACE) {
            Ok(()) => {},
            Err(Errno::NOENT | Errno::EXIST) => continue,
            Err(error) => {
                return Err(format!(
                    "cannot claim {label} {} for removal: {error}",
                    path.display()
                ));
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn restore_claimed(
    parent: &OwnedFd,
    claimed: &std::ffi::OsStr,
    target: &std::ffi::OsStr,
    claimed_path: &Path,
    path: &Path,
    label: &str,
    sync_parent: &mut impl FnMut(&OwnedFd) -> Result<(), String>,
    error: String,
) -> String {
    match renameat_with(parent, claimed, parent, target, RenameFlags::NOREPLACE) {
        Ok(()) => match sync_parent(parent) {
            Ok(()) => error,
            Err(sync_error) => format!(
                "{error}; restored {label} {} but cannot sync its parent: {sync_error}",
                path.display()
            ),
        },
        Err(Errno::EXIST) => format!(
            "{error}; preserved the claimed file at {} because {} was recreated",
            claimed_path.display(),
            path.display()
        ),
        Err(restore_error) => format!(
            "{error}; cannot restore {label} {}: {restore_error}",
            path.display()
        ),
    }
}

fn exact_sha256(path: &Path, expected_hash: &str, bytes: &[u8], label: &str) -> Result<(), String> {
    let actual_hash = format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    if actual_hash == expected_hash {
        Ok(())
    } else {
        Err(format!(
            "{label} {} hash changed: expected {expected_hash}, found {actual_hash}",
            path.display()
        ))
    }
}

fn publish_new_or_exact_with(
    path: &Path,
    expected: &[u8],
    limit: usize,
    label: &str,
    write_prepared: impl FnOnce(&mut File, &[u8]) -> Result<(), String>,
) -> Result<bool, String> {
    publish_new_or_exact_with_hooks(path, expected, limit, label, write_prepared, |parent| {
        sync_directory(parent, label)
    })
}

fn publish_new_or_exact_with_hooks(
    path: &Path,
    expected: &[u8],
    limit: usize,
    label: &str,
    write_prepared: impl FnOnce(&mut File, &[u8]) -> Result<(), String>,
    mut sync_parent: impl FnMut(&OwnedFd) -> Result<(), String>,
) -> Result<bool, String> {
    if expected.len() > limit {
        return Err(format!("{label} exceeds the {limit}-byte limit"));
    }
    let parent_path = path
        .parent()
        .ok_or_else(|| format!("{label} path {} has no parent", path.display()))?;
    let target = path
        .file_name()
        .ok_or_else(|| format!("{label} path {} has no file name", path.display()))?;
    let parent = open_directory(parent_path, label)?;
    if let Some(actual) = read_regular_at(&parent, target, path, limit, label)? {
        exact_bytes(path, expected, &actual, label)?;
        sync_parent(&parent)?;
        return Ok(false);
    }

    let (temporary, fd) = create_temporary(&parent, target, path, label)?;
    let mut file = File::from(fd);
    write_prepared(&mut file, expected)?;
    drop(file);

    match renameat_with(&parent, &temporary, &parent, target, RenameFlags::NOREPLACE) {
        Ok(()) => {
            sync_parent(&parent)?;
            Ok(true)
        },
        Err(Errno::EXIST) => {
            let actual =
                read_regular_at(&parent, target, path, limit, label)?.ok_or_else(|| {
                    format!("{label} {} disappeared during publication", path.display())
                })?;
            exact_bytes(path, expected, &actual, label)?;
            let _ = unlinkat(&parent, &temporary, AtFlags::empty());
            sync_parent(&parent)?;
            Ok(false)
        },
        Err(error) => Err(format!(
            "cannot publish {label} {}: {error}",
            path.display()
        )),
    }
}

fn create_temporary(
    parent: &OwnedFd,
    target: &std::ffi::OsStr,
    display_path: &Path,
    label: &str,
) -> Result<(OsString, OwnedFd), String> {
    for _ in 0..1024 {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let mut temporary = OsString::from(".");
        temporary.push(target);
        temporary.push(format!(".yo-prepare-{}-{sequence}", std::process::id()));
        match openat(parent, &temporary, CREATE_FLAGS, Mode::from_raw_mode(0o600)) {
            Ok(fd) => return Ok((temporary, fd)),
            Err(Errno::EXIST) => continue,
            Err(error) => {
                return Err(format!(
                    "cannot prepare {label} {}: {error}",
                    display_path.display()
                ));
            },
        }
    }
    Err(format!(
        "cannot allocate a unique prepared {label} for {}",
        display_path.display()
    ))
}

pub(crate) fn open_directory(path: &Path, label: &str) -> Result<OwnedFd, String> {
    let mut directory = if path.is_absolute() {
        open(Path::new("/"), DIRECTORY_FLAGS, Mode::empty())
    } else {
        open(Path::new("."), DIRECTORY_FLAGS, Mode::empty())
    }
    .map_err(|error| format!("cannot open {label} directory anchor: {error}"))?;
    for component in path.components() {
        let name = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::ParentDir => std::ffi::OsStr::new(".."),
            Component::Normal(name) => name,
            Component::Prefix(_) => {
                return Err(format!(
                    "cannot open {label} directory {} with a platform prefix",
                    path.display()
                ));
            },
        };
        directory = openat(&directory, name, DIRECTORY_FLAGS, Mode::empty()).map_err(|error| {
            format!(
                "cannot open {label} directory {} without symlinks: {error}",
                path.display()
            )
        })?;
    }
    Ok(directory)
}

pub(crate) fn ensure_directory(path: &Path, label: &str) -> Result<(), String> {
    let mut directory = if path.is_absolute() {
        open(Path::new("/"), DIRECTORY_FLAGS, Mode::empty())
    } else {
        open(Path::new("."), DIRECTORY_FLAGS, Mode::empty())
    }
    .map_err(|error| format!("cannot open {label} directory anchor: {error}"))?;
    for component in path.components() {
        let name = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::ParentDir => std::ffi::OsStr::new(".."),
            Component::Normal(name) => name,
            Component::Prefix(_) => {
                return Err(format!(
                    "cannot create {label} directory {} with a platform prefix",
                    path.display()
                ));
            },
        };
        directory = match openat(&directory, name, DIRECTORY_FLAGS, Mode::empty()) {
            Ok(next) => next,
            Err(Errno::NOENT) if !matches!(component, Component::ParentDir) => {
                match mkdirat(&directory, name, Mode::from_raw_mode(0o777)) {
                    Ok(()) | Err(Errno::EXIST) => {},
                    Err(error) => {
                        return Err(format!(
                            "cannot create {label} directory {}: {error}",
                            path.display()
                        ));
                    },
                }
                openat(&directory, name, DIRECTORY_FLAGS, Mode::empty()).map_err(|error| {
                    format!(
                        "cannot open created {label} directory {} without symlinks: {error}",
                        path.display()
                    )
                })?
            },
            Err(error) => {
                return Err(format!(
                    "cannot open {label} directory {} without symlinks: {error}",
                    path.display()
                ));
            },
        };
    }
    Ok(())
}

pub(crate) fn sync_directory(directory: &OwnedFd, label: &str) -> Result<(), String> {
    File::from(
        rustix::io::dup(directory)
            .map_err(|error| format!("cannot retain {label} parent for sync: {error}"))?,
    )
    .sync_all()
    .map_err(|error| format!("cannot sync {label} parent: {error}"))
}

fn exact_bytes(path: &Path, expected: &[u8], actual: &[u8], label: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} {} already contains different bytes",
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests;
