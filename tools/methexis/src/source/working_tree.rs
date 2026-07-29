//! Symlink-safe immutable capture of working-tree code Sources.

use std::{
    ffi::OsStr,
    fs::File,
    io::Read,
    os::fd::OwnedFd,
    path::{Component, Path},
};

use rustix::{
    fs::{FileType, Mode, OFlags, fstat, open, openat},
    io::Errno,
};
use sha2::{Digest, Sha256};

use super::freshness::FreshnessFailure;
use crate::file_identity::FileIdentity;

const MAX_CODE_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE_RECORD_BYTES: usize = 256 * 1024;
const OPEN_DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(super) enum CaptureState {
    Fresh(Capture),
    Stale {
        reason: &'static str,
        capture: Capture,
    },
    Invalid {
        reason: &'static str,
        capture: Capture,
    },
}

pub(crate) enum Capture {
    File {
        path: String,
        identity: FileIdentity,
        hash: String,
    },
    Missing {
        path: String,
    },
    Invalid {
        path: String,
    },
}

pub(super) fn capture(
    repository_root: &Path,
    relative_path: &str,
    expected_hash: &str,
) -> Result<CaptureState, FreshnessFailure> {
    let mut file = match open_file(repository_root, Path::new(relative_path)) {
        Ok(file) => file,
        Err(OpenFailure::Missing) => {
            return Ok(CaptureState::Stale {
                reason: "code_source_missing",
                capture: Capture::Missing {
                    path: relative_path.to_owned(),
                },
            });
        },
        Err(OpenFailure::Unsafe) => {
            return Ok(CaptureState::Invalid {
                reason: "code_source_path_invalid",
                capture: Capture::Invalid {
                    path: relative_path.to_owned(),
                },
            });
        },
        Err(OpenFailure::Io(message)) => {
            return Err(failure(
                "source_capture_failed",
                message,
                vec![relative_path.to_owned()],
            ));
        },
    };
    let before = FileIdentity::capture(&file).map_err(|error| {
        failure(
            "source_capture_failed",
            error.to_string(),
            vec![relative_path.to_owned()],
        )
    })?;
    let bytes = read_bounded(
        &mut file,
        relative_path,
        MAX_CODE_SOURCE_BYTES,
        "code Source",
    )?;
    let after = FileIdentity::capture(&file).map_err(|error| {
        failure(
            "source_capture_failed",
            error.to_string(),
            vec![relative_path.to_owned()],
        )
    })?;
    if before != after {
        return Err(changed(relative_path));
    }
    let hash = hash_bytes(&bytes);
    if hash != expected_hash {
        return Ok(CaptureState::Stale {
            reason: "code_hash_mismatch",
            capture: Capture::File {
                path: relative_path.to_owned(),
                identity: after,
                hash,
            },
        });
    }
    Ok(CaptureState::Fresh(Capture::File {
        path: relative_path.to_owned(),
        identity: after,
        hash,
    }))
}

pub(super) fn final_revalidate(
    repository_root: &Path,
    capture: &Capture,
) -> Result<(), FreshnessFailure> {
    final_revalidate_with(repository_root, capture, || {})
}

fn final_revalidate_with(
    repository_root: &Path,
    capture: &Capture,
    mut after_read: impl FnMut(),
) -> Result<(), FreshnessFailure> {
    match capture {
        Capture::File {
            path,
            identity: captured_identity,
            hash,
        } => {
            let mut file =
                open_file(repository_root, Path::new(path)).map_err(|_| changed(path))?;
            let before = FileIdentity::capture(&file).map_err(|_| changed(path))?;
            let bytes = read_bounded(&mut file, path, MAX_CODE_SOURCE_BYTES, "code Source")
                .map_err(|_| changed(path))?;
            after_read();
            let after = FileIdentity::capture(&file).map_err(|_| changed(path))?;
            let mut current =
                open_file(repository_root, Path::new(path)).map_err(|_| changed(path))?;
            let current_before = FileIdentity::capture(&current).map_err(|_| changed(path))?;
            let current_bytes =
                read_bounded(&mut current, path, MAX_CODE_SOURCE_BYTES, "code Source")
                    .map_err(|_| changed(path))?;
            let current_after = FileIdentity::capture(&current).map_err(|_| changed(path))?;
            if before != after
                || after != *captured_identity
                || current_before != current_after
                || current_after != *captured_identity
                || hash_bytes(&bytes) != *hash
                || hash_bytes(&current_bytes) != *hash
            {
                return Err(changed(path));
            }
        },
        Capture::Missing { path } => match open_file(repository_root, Path::new(path)) {
            Err(OpenFailure::Missing) => {},
            Ok(_) | Err(OpenFailure::Unsafe | OpenFailure::Io(_)) => return Err(changed(path)),
        },
        Capture::Invalid { path } => match open_file(repository_root, Path::new(path)) {
            Err(OpenFailure::Unsafe) => {},
            Ok(_) | Err(OpenFailure::Missing | OpenFailure::Io(_)) => return Err(changed(path)),
        },
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn final_revalidate_after_read(
    repository_root: &Path,
    capture: &Capture,
    after_read: impl FnMut(),
) -> Result<(), FreshnessFailure> {
    final_revalidate_with(repository_root, capture, after_read)
}

fn open_file(repository_root: &Path, relative: &Path) -> Result<File, OpenFailure> {
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(OpenFailure::Unsafe);
    }
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
    require_regular(&fd)?;
    Ok(File::from(fd))
}

pub(crate) fn capture_record(
    repository_root: &Path,
    relative_path: &str,
) -> Result<(Vec<u8>, Capture), FreshnessFailure> {
    let mut file = open_file(repository_root, Path::new(relative_path)).map_err(|error| {
        failure(
            "source_record_capture_failed",
            format!("cannot capture Source record `{relative_path}`: {error:?}"),
            vec![relative_path.to_owned()],
        )
    })?;
    let before = FileIdentity::capture(&file).map_err(|error| {
        failure(
            "source_record_capture_failed",
            error.to_string(),
            vec![relative_path.to_owned()],
        )
    })?;
    let bytes = read_bounded(
        &mut file,
        relative_path,
        MAX_SOURCE_RECORD_BYTES,
        "Source record",
    )?;
    let after = FileIdentity::capture(&file).map_err(|error| {
        failure(
            "source_record_capture_failed",
            error.to_string(),
            vec![relative_path.to_owned()],
        )
    })?;
    if before != after {
        return Err(changed(relative_path));
    }
    let hash = hash_bytes(&bytes);
    Ok((
        bytes,
        Capture::File {
            path: relative_path.to_owned(),
            identity: after,
            hash,
        },
    ))
}

fn require_regular(file: &OwnedFd) -> Result<(), OpenFailure> {
    let stat = fstat(file).map_err(|error| OpenFailure::Io(error.to_string()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(OpenFailure::Unsafe);
    }
    Ok(())
}

fn read_bounded(
    file: &mut File,
    path: &str,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, FreshnessFailure> {
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            failure(
                "source_capture_failed",
                error.to_string(),
                vec![path.to_owned()],
            )
        })?;
    if bytes.len() > limit {
        return Err(failure(
            "source_too_large",
            format!("{label} exceeds the Pilot size limit"),
            vec![path.to_owned()],
        ));
    }
    Ok(bytes)
}

fn hash_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn changed(path: &str) -> FreshnessFailure {
    failure(
        "source_changed_during_validation",
        format!("code Source `{path}` changed during validation"),
        vec![path.to_owned()],
    )
}

fn failure(code: &'static str, message: String, affected_ids: Vec<String>) -> FreshnessFailure {
    FreshnessFailure {
        code,
        message,
        affected_ids,
    }
}

#[derive(Debug)]
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
            "cannot open code Source component `{}`: {error}",
            component.to_string_lossy()
        )),
    }
}
