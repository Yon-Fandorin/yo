use std::path::Path;

use super::{MAX_INPUT_BYTES, MAX_PACKET_BYTES};
use crate::{
    bounded_file,
    review_protocol::{Captured, NamedCaptured, digest, relative},
};

pub(super) fn capture_file(path: &Path, label: &str) -> Result<Captured, String> {
    let bytes = bounded_file::read_regular(path, MAX_INPUT_BYTES, label)?;
    captured(path.to_string_lossy().into_owned(), bytes)
}

pub(super) fn capture_packet(path: &Path, label: &str) -> Result<Captured, String> {
    let bytes = bounded_file::read_regular(path, MAX_PACKET_BYTES, label)?;
    std::str::from_utf8(&bytes).map_err(|_| {
        format!(
            "review delta input `{}` is not UTF-8 model-visible text",
            path.display()
        )
    })?;
    Ok(Captured {
        path: path.to_string_lossy().into_owned(),
        hash: digest(&bytes),
        bytes,
    })
}

pub(super) fn capture_published(
    repository: &Path,
    path: &Path,
    label: &str,
    maximum: usize,
) -> Result<Captured, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("cannot resolve {label} {}: {error}", path.display()))?;
    let bytes = bounded_file::read_regular(&canonical, maximum, label)?;
    std::str::from_utf8(&bytes).map_err(|_| format!("{label} is not UTF-8 model-visible text"))?;
    Ok(Captured {
        path: relative(repository, &canonical),
        hash: digest(&bytes),
        bytes,
    })
}

pub(super) fn require_current_file(
    path: &Path,
    expected: &Captured,
    label: &str,
) -> Result<(), String> {
    let actual = capture_file(path, label)?;
    if actual.hash == expected.hash && actual.bytes == expected.bytes {
        Ok(())
    } else {
        Err(format!("{label} changed during review delta construction"))
    }
}

pub(super) fn require_current_packet(
    path: &Path,
    expected: &Captured,
    label: &str,
) -> Result<(), String> {
    let actual = capture_packet(path, label)?;
    if actual.hash == expected.hash && actual.bytes == expected.bytes {
        Ok(())
    } else {
        Err(format!("{label} changed during review delta construction"))
    }
}

pub(super) fn require_named_captures(
    actual: &[NamedCaptured],
    expected: &[NamedCaptured],
) -> Result<(), String> {
    if actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(left, right)| {
            left.name == right.name
                && left.artifact.path == right.artifact.path
                && left.artifact.hash == right.artifact.hash
                && left.artifact.bytes == right.artifact.bytes
        })
    {
        Ok(())
    } else {
        Err("validation evidence changed during review delta construction".to_owned())
    }
}

pub(super) fn captured(path: String, bytes: Vec<u8>) -> Result<Captured, String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "review delta input `{path}` exceeds the {MAX_INPUT_BYTES}-byte limit"
        ));
    }
    std::str::from_utf8(&bytes)
        .map_err(|_| format!("review delta input `{path}` is not UTF-8 model-visible text"))?;
    Ok(Captured {
        path,
        hash: digest(&bytes),
        bytes,
    })
}

pub(super) fn require_hash(value: &str, label: &str) -> Result<(), String> {
    let valid = value.strip_prefix("sha256:").is_some_and(|suffix| {
        suffix.len() == 64
            && suffix
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    });
    if valid {
        Ok(())
    } else {
        Err(format!("{label} must be a canonical SHA-256 identity"))
    }
}

pub(super) fn require_exact_hash(expected: &str, bytes: &[u8], label: &str) -> Result<(), String> {
    require_hash(expected, label)?;
    let actual = digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash mismatch: expected {expected}, found {actual}"
        ))
    }
}
