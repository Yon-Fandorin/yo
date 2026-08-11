use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Component, Path},
    process::ExitCode,
};

use super::{
    MAX_INPUT_BYTES, MAX_REQUEST_BYTES,
    model::{
        CheckpointIdentity, ContextManifest, ContextResult, EvidenceRequest, TOKENIZER_PROFILE,
    },
    trusted_git::trusted_git_bytes,
};
use crate::{
    bounded_file,
    review_protocol::{Captured, NamedCaptured, digest, resolve_input_path, sorted_unique},
};

pub(super) struct ContextCapture {
    pub(super) result: ContextResult,
    pub(super) request: Captured,
    pub(super) context: Captured,
    pub(super) manifest: Captured,
    pub(super) active_checkpoint: CheckpointIdentity,
    pub(super) included_ids: Vec<String>,
}

pub(super) struct Inputs {
    pub(super) base_commit: String,
    pub(super) candidate_commit: String,
    pub(super) diff: Captured,
    pub(super) context: ContextCapture,
    pub(super) authorities: Vec<Captured>,
    pub(super) slice_contract: Captured,
    pub(super) validation: Vec<NamedCaptured>,
    pub(super) lenses: Vec<String>,
    pub(super) questions: Vec<String>,
    pub(super) required_knowledge_ids: Vec<String>,
    pub(super) delivery_profile_bytes: Vec<u8>,
    pub(super) max_tokens: usize,
}

pub(super) fn capture_context(
    repository: &Path,
    request_path: &Path,
) -> Result<ContextCapture, String> {
    let request = capture_context_request(repository, request_path)?;
    capture_context_with_request(repository, request_path, request)
}

pub(super) fn capture_context_request(
    repository: &Path,
    request_path: &Path,
) -> Result<Captured, String> {
    let relative = request_path.strip_prefix(repository).map_err(|_| {
        "Methexis ContextBuild request must be inside the candidate worktree".to_owned()
    })?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(
            "Methexis ContextBuild request must use a direct path inside the candidate worktree"
                .to_owned(),
        );
    }
    let request_bytes = bounded_file::read_regular(
        request_path,
        MAX_REQUEST_BYTES,
        "Methexis ContextBuild request",
    )?;
    captured(request_path.to_string_lossy().into_owned(), request_bytes)
}

pub(super) fn capture_context_with_request(
    repository: &Path,
    request_path: &Path,
    request: Captured,
) -> Result<ContextCapture, String> {
    let result = resolve_context(request_path)?;
    if result.schema != "methexis.context-result/v1alpha1"
        || !result.ok
        || result.operation != "resolve_context"
        || result.authority != "trusted_integration"
    {
        return Err("Methexis returned a non-success ContextBuild result".to_owned());
    }
    let context_path = repository.join(&result.context.path);
    let manifest_path = repository.join(&result.manifest.path);
    let context_bytes = bounded_file::read_regular(
        &context_path,
        MAX_INPUT_BYTES,
        "Methexis ContextBuild context",
    )?;
    let manifest_bytes = bounded_file::read_regular(
        &manifest_path,
        MAX_INPUT_BYTES,
        "Methexis ContextBuild manifest",
    )?;
    require_hash(&result.context.hash, &context_bytes, "ContextBuild context")?;
    require_hash(
        &result.manifest.hash,
        &manifest_bytes,
        "ContextBuild manifest",
    )?;
    let manifest: ContextManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid ContextBuild manifest: {error}"))?;
    if manifest.schema != "methexis.context-manifest/v1alpha1"
        || manifest.build_id != result.build_id
        || manifest.context.hash != result.context.hash
        || manifest.context.path != "context.md"
        || manifest.plan.tokenizer_profile != TOKENIZER_PROFILE
    {
        return Err("ContextBuild result and manifest identities differ".to_owned());
    }
    Ok(ContextCapture {
        result,
        request,
        context: captured(context_path.to_string_lossy().into_owned(), context_bytes)?,
        manifest: captured(manifest_path.to_string_lossy().into_owned(), manifest_bytes)?,
        active_checkpoint: manifest.plan.checkpoint,
        included_ids: manifest
            .plan
            .units
            .into_iter()
            .map(|unit| unit.id)
            .collect(),
    })
}

fn resolve_context(request_path: &Path) -> Result<ContextResult, String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let arguments = [
        OsString::from("resolve-context"),
        request_path.as_os_str().to_owned(),
    ];
    let code = methexis::run(arguments, &mut stdout, &mut stderr)
        .map_err(|error| format!("cannot run Methexis ContextBuild: {error}"))?;
    if code != ExitCode::SUCCESS {
        return Err(format!(
            "Methexis ContextBuild failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    serde_json::from_slice(&stdout)
        .map_err(|error| format!("invalid Methexis ContextBuild result: {error}"))
}

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
            let header = std::str::from_utf8(header)
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
    requests: &[EvidenceRequest],
) -> Result<Vec<NamedCaptured>, String> {
    let mut names = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut captured_inputs = Vec::new();
    for request in requests {
        if request.name.trim().is_empty() || !names.insert(request.name.clone()) {
            return Err("validation evidence names must be non-empty and unique".to_owned());
        }
        let path = resolve_input_path(repository, &request.path);
        let bytes = bounded_file::read_regular(&path, MAX_INPUT_BYTES, "validation evidence")?;
        let canonical = std::fs::canonicalize(&path).map_err(|error| {
            format!(
                "cannot resolve validation evidence path {}: {error}",
                path.display()
            )
        })?;
        if !paths.insert(canonical) {
            return Err("validation evidence paths must be unique".to_owned());
        }
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
    std::str::from_utf8(&bytes)
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
