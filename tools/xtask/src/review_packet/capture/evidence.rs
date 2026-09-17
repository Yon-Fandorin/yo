use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
    str,
};

use super::super::{MAX_INPUT_BYTES, model::EvidenceRequest, trusted_git::trusted_git_bytes};
use crate::{
    bounded_file,
    review_protocol::{Captured, NamedCaptured, digest, resolve_input_path, sorted_unique},
    validation_summary,
};

pub(super) fn capture_diff(
    repository: &Path,
    base: &str,
    candidate: &str,
) -> Result<Vec<u8>, String> {
    trusted_git_bytes(
        repository,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            base,
            candidate,
            "--",
        ],
    )
}

pub(super) fn capture_authorities(
    repository: &Path,
    candidate: &str,
    paths: &[String],
) -> Result<Vec<Captured>, String> {
    let paths = sorted_unique(paths, "repository authority path")?;
    paths
        .into_iter()
        .map(|path| {
            require_repository_path(&path)?;
            let listing = trusted_git_bytes(
                repository,
                &["ls-tree", "-z", "--full-tree", candidate, "--", &path],
            )?;
            let entry = listing
                .strip_suffix(&[0])
                .ok_or_else(|| format!("authority `{path}` has no exact Git tree entry"))?;
            let separator = entry
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or_else(|| format!("authority `{path}` has an invalid Git tree entry"))?;
            let (header, listed_path) = (&entry[..separator], &entry[separator + 1..]);
            if listed_path != path.as_bytes() {
                return Err(format!("authority `{path}` did not resolve exactly"));
            }
            let header = str::from_utf8(header)
                .map_err(|error| format!("invalid authority tree entry: {error}"))?;
            let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 || fields[0] != "100644" || fields[1] != "blob" {
                return Err(format!(
                    "authority `{path}` must be a non-executable regular Git blob"
                ));
            }
            let bytes = trusted_git_bytes(repository, &["cat-file", "blob", fields[2]])?;
            captured(path, bytes)
        })
        .collect()
}

pub(super) fn capture_validation(
    repository: &Path,
    candidate_commit: &str,
    requests: &[EvidenceRequest],
) -> Result<Vec<NamedCaptured>, String> {
    let mut names = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut pending = Vec::new();
    for request in requests {
        if request.name.trim().is_empty() || !names.insert(request.name.clone()) {
            return Err("validation evidence names must be non-empty and unique".to_owned());
        }
        let path = resolve_input_path(repository, &request.path);
        let canonical = fs::canonicalize(&path).map_err(|error| {
            format!(
                "cannot resolve validation evidence path {}: {error}",
                path.display()
            )
        })?;
        if !paths.insert(canonical) {
            return Err("validation evidence paths must be unique".to_owned());
        }
        pending.push((request, path));
    }
    let mut captured_inputs = Vec::new();
    for (request, path) in pending {
        let bytes = bounded_file::read_regular(&path, MAX_INPUT_BYTES, "validation evidence")?;
        validation_summary::verify_review_input(
            repository,
            &bytes,
            &request.name,
            candidate_commit,
        )
        .map_err(|error| {
            format!(
                "invalid validation evidence for `{}`: {error}",
                request.name
            )
        })?;
        captured_inputs.push(NamedCaptured {
            name: request.name.clone(),
            artifact: captured(path.to_string_lossy().into_owned(), bytes)?,
        });
    }
    captured_inputs.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(captured_inputs)
}

pub(super) fn captured(path: String, bytes: Vec<u8>) -> Result<Captured, String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "review input `{path}` exceeds the {MAX_INPUT_BYTES}-byte limit"
        ));
    }
    str::from_utf8(&bytes)
        .map_err(|_| format!("review input `{path}` is not UTF-8 model-visible text"))?;
    Ok(Captured {
        path,
        hash: digest(&bytes),
        bytes,
    })
}

pub(super) fn same_capture(left: &Captured, right: &Captured) -> bool {
    left.path == right.path && left.hash == right.hash && left.bytes == right.bytes
}

pub(super) fn same_captures(actual: &[Captured], expected: &[Captured]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(left, right)| same_capture(left, right))
}

pub(super) fn same_named_captures(actual: &[NamedCaptured], expected: &[NamedCaptured]) -> bool {
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(left, right)| {
            left.name == right.name && same_capture(&left.artifact, &right.artifact)
        })
}

pub(super) fn require_repository_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            !matches!(component, Component::Normal(_))
                || matches!(component, Component::Normal(value) if value.to_string_lossy().contains(':'))
        })
    {
        return Err("repository authority paths must be safe relative paths".to_owned());
    }
    Ok(())
}

pub(super) fn require_hash(expected: &str, bytes: &[u8], label: &str) -> Result<(), String> {
    let actual = digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash mismatch: expected {expected}, found {actual}"
        ))
    }
}
