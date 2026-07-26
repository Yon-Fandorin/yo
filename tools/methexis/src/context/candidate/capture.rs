//! Symlink-safe, bounded, immutable candidate-file capture.

use std::{
    ffi::OsStr,
    fs::File,
    io::Read,
    path::{Component, Path},
};

use rustix::{
    fs::{FileType, Mode, OFlags, fstat, open, openat},
    io::Errno,
};

use crate::context::{hash::digest, wire::ResolveFailure};

const MAX_CANDIDATE_BYTES: usize = 4 * 1024 * 1024;
const OPEN_DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(super) struct CapturedFile {
    pub(super) path: String,
    pub(super) bytes: Vec<u8>,
    pub(super) hash: String,
    identity: Identity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Identity {
    device: u64,
    inode: u64,
    size: i64,
    modified_seconds: i64,
    modified_nanoseconds: u64,
}

pub(super) fn capture(
    repository_root: &Path,
    relative_path: &str,
) -> Result<CapturedFile, ResolveFailure> {
    let mut file = open_file(repository_root, Path::new(relative_path))
        .map_err(|error| open_failure(error, relative_path))?;
    let before = identity(&file).map_err(|error| io_failure(error, relative_path))?;
    let bytes = read_bounded(&mut file, relative_path)?;
    let after = identity(&file).map_err(|error| io_failure(error, relative_path))?;
    if before != after {
        return Err(changed(relative_path));
    }
    Ok(CapturedFile {
        path: relative_path.to_owned(),
        hash: digest(&bytes),
        bytes,
        identity: after,
    })
}

pub(super) fn final_revalidate(
    repository_root: &Path,
    capture: &CapturedFile,
) -> Result<(), ResolveFailure> {
    final_revalidate_with(repository_root, capture, || {})
}

fn final_revalidate_with(
    repository_root: &Path,
    capture: &CapturedFile,
    after_read: impl FnOnce(),
) -> Result<(), ResolveFailure> {
    let mut file =
        open_file(repository_root, Path::new(&capture.path)).map_err(|_| changed(&capture.path))?;
    let before = identity(&file).map_err(|_| changed(&capture.path))?;
    let bytes = read_bounded(&mut file, &capture.path).map_err(|_| changed(&capture.path))?;
    after_read();
    let after = identity(&file).map_err(|_| changed(&capture.path))?;
    let mut reopened =
        open_file(repository_root, Path::new(&capture.path)).map_err(|_| changed(&capture.path))?;
    let reopened_before = identity(&reopened).map_err(|_| changed(&capture.path))?;
    let reopened_bytes =
        read_bounded(&mut reopened, &capture.path).map_err(|_| changed(&capture.path))?;
    let reopened_after = identity(&reopened).map_err(|_| changed(&capture.path))?;
    if before != after
        || after != capture.identity
        || reopened_before != reopened_after
        || reopened_after != capture.identity
        || digest(&bytes) != capture.hash
        || digest(&reopened_bytes) != capture.hash
    {
        return Err(changed(&capture.path));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn final_revalidate_after_read(
    repository_root: &Path,
    capture: &CapturedFile,
    after_read: impl FnOnce(),
) -> Result<(), ResolveFailure> {
    final_revalidate_with(repository_root, capture, after_read)
}

fn open_file(repository_root: &Path, relative: &Path) -> Result<File, OpenFailure> {
    if !safe_relative(relative) {
        return Err(OpenFailure::Unsafe);
    }
    let components = relative.components().collect::<Vec<_>>();
    let mut directory = open(repository_root, OPEN_DIRECTORY, Mode::empty())
        .map_err(|error| classify(error, repository_root.as_os_str()))?;
    for component in components.iter().take(components.len() - 1) {
        let Component::Normal(name) = component else {
            return Err(OpenFailure::Unsafe);
        };
        directory = openat(&directory, *name, OPEN_DIRECTORY, Mode::empty())
            .map_err(|error| classify(error, name))?;
    }
    let Component::Normal(name) = components[components.len() - 1] else {
        return Err(OpenFailure::Unsafe);
    };
    let fd = openat(
        &directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| classify(error, name))?;
    let stat = fstat(&fd).map_err(|error| OpenFailure::Io(error.to_string()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(OpenFailure::Unsafe);
    }
    Ok(File::from(fd))
}

pub(super) fn safe_relative(path: &Path) -> bool {
    let Some(raw) = path.to_str() else {
        return false;
    };
    if raw.contains('\\')
        || raw
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return false;
    }
    let mut count = 0usize;
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return false;
        }
        count += 1;
    }
    count > 0
}

fn identity(file: &File) -> std::io::Result<Identity> {
    let stat = fstat(file).map_err(std::io::Error::from)?;
    Ok(Identity {
        device: stat.st_dev,
        inode: stat.st_ino,
        size: stat.st_size,
        modified_seconds: stat.st_mtime,
        modified_nanoseconds: stat.st_mtime_nsec,
    })
}

fn read_bounded(file: &mut File, path: &str) -> Result<Vec<u8>, ResolveFailure> {
    let mut bytes = Vec::new();
    file.take((MAX_CANDIDATE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_failure(error, path))?;
    if bytes.len() > MAX_CANDIDATE_BYTES {
        return Err(failure(
            "candidate_too_large",
            "candidate result exceeds the Pilot size limit",
            false,
            path,
        ));
    }
    Ok(bytes)
}

fn changed(path: &str) -> ResolveFailure {
    failure(
        "candidate_changed_during_resolution",
        "candidate result changed during context resolution",
        true,
        path,
    )
}

fn io_failure(error: impl std::fmt::Display, path: &str) -> ResolveFailure {
    failure("candidate_capture_failed", &error.to_string(), false, path)
}

fn open_failure(error: OpenFailure, path: &str) -> ResolveFailure {
    match error {
        OpenFailure::Unsafe => failure(
            "candidate_path_invalid",
            "candidate path must name a regular file beneath the repository without symlinks",
            false,
            path,
        ),
        OpenFailure::Missing => failure(
            "candidate_unreadable",
            "candidate result does not exist",
            false,
            path,
        ),
        OpenFailure::Io(message) => failure("candidate_unreadable", &message, false, path),
    }
}

fn failure(code: &str, message: &str, retryable: bool, path: &str) -> ResolveFailure {
    ResolveFailure::new(
        None,
        code,
        message,
        retryable,
        Vec::new(),
        vec![path.to_owned()],
        "correct or regenerate the Librarian candidate result and retry",
    )
}

enum OpenFailure {
    Missing,
    Unsafe,
    Io(String),
}

fn classify(error: Errno, component: &OsStr) -> OpenFailure {
    match error {
        Errno::NOENT => OpenFailure::Missing,
        Errno::LOOP | Errno::NOTDIR => OpenFailure::Unsafe,
        error => OpenFailure::Io(format!(
            "cannot open candidate component `{}`: {error}",
            component.to_string_lossy()
        )),
    }
}
