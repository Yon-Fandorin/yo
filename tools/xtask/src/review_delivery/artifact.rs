use std::{fs::File, io::Read, path::Path};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    REQUEST_LIMIT,
    model::{Artifact, Route},
    usage::{self, UsageBinding},
};
use crate::{bounded_file, review_egress::AuthorizedDelivery, review_protocol::digest};

pub(super) fn route(delivery: &AuthorizedDelivery) -> Route<'_> {
    Route {
        provider: &delivery.provider,
        account: &delivery.account,
        model: &delivery.model,
    }
}

pub(super) fn publish_provider_usage(
    session_root: &Path,
    output_directory: &Path,
    binding: UsageBinding,
) -> Result<Artifact, String> {
    let document = usage::project(session_root, binding)?;
    let bytes = canonical_json(&document)?;
    let path = output_directory.join("provider-usage.json");
    publish_exact(
        &path,
        &bytes,
        REQUEST_LIMIT,
        "external review Provider Usage binding",
    )?;
    Ok(artifact(&path, &bytes, true))
}

pub(super) fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot encode review delivery artifact: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn publish_exact(
    path: &Path,
    bytes: &[u8],
    limit: usize,
    label: &str,
) -> Result<(), String> {
    if bounded_file::publish_new_or_exact(path, bytes, limit, label)? {
        Ok(())
    } else {
        Err(format!(
            "{label} already exists at {}; refusing to reuse a completed delivery path",
            path.display()
        ))
    }
}

pub(super) fn publish_claim(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bounded_file::publish_new_or_exact(
        path,
        bytes,
        REQUEST_LIMIT,
        "external review delivery claim",
    )? {
        Ok(())
    } else {
        Err(format!(
            "external review delivery is already claimed at {}; refusing another provider request",
            path.display()
        ))
    }
}

pub(super) fn artifact(path: &Path, bytes: &[u8], published: bool) -> Artifact {
    Artifact {
        path: path.to_string_lossy().into_owned(),
        hash: digest(bytes),
        bytes: bytes.len(),
        published,
    }
}

pub(super) fn require_exact_file_hash(
    path: &Path,
    expected: &str,
    limit: usize,
    label: &str,
) -> Result<(), String> {
    let bytes = bounded_file::read_regular(path, limit, label)?;
    let actual = digest(&bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash mismatch: expected {expected}, found {actual}"
        ))
    }
}

pub(super) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("cannot open current-develop yo binary: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot hash current-develop yo binary: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!(
        "sha256:{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

pub(super) fn compact_path(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') {
        Err(format!(
            "{label} must be a non-empty path of at most 4096 bytes"
        ))
    } else {
        Ok(())
    }
}

pub(super) fn require_sha256(value: &str, label: &str) -> Result<(), String> {
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

pub(super) fn combine_failures(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (None, None) => None,
        (Some(error), None) | (None, Some(error)) => Some(error),
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
    }
}
