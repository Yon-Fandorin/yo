use std::{
    ffi::OsString,
    path::{Component, Path},
    process::ExitCode,
};

use super::{
    super::{
        MAX_INPUT_BYTES, MAX_REQUEST_BYTES,
        model::{ContextManifest, ContextResult, TOKENIZER_PROFILE},
    },
    ContextCapture, evidence,
};
use crate::bounded_file;

pub(in crate::review_packet) fn capture_context(
    repository: &Path,
    request_path: &Path,
) -> Result<ContextCapture, String> {
    let request = capture_context_request(repository, request_path)?;
    capture_context_with_request(repository, request_path, request)
}

pub(in crate::review_packet) fn capture_context_request(
    repository: &Path,
    request_path: &Path,
) -> Result<crate::review_protocol::Captured, String> {
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
    evidence::captured(request_path.to_string_lossy().into_owned(), request_bytes)
}

pub(in crate::review_packet) fn capture_context_with_request(
    repository: &Path,
    request_path: &Path,
    request: crate::review_protocol::Captured,
) -> Result<ContextCapture, String> {
    let result = resolve_context(request_path)?;
    if result.schema != "methexis.context-result/v1alpha1"
        || !result.ok
        || result.operation != "resolve_context"
        || result.authority != "trusted_integration"
        || result.checkpoint.is_some()
        || result.activation_request.is_some()
        || result.predecessor_active_record_hash.is_some()
        || result.proposed_active_record_hash.is_some()
    {
        return Err("Methexis returned a non-success ContextBuild result".to_owned());
    }
    capture_context_artifacts(repository, request, result)
}

pub(super) fn capture_context_artifacts(
    repository: &Path,
    request: crate::review_protocol::Captured,
    result: ContextResult,
) -> Result<ContextCapture, String> {
    evidence::require_repository_path(&result.context.path).map_err(|_| {
        "ContextBuild result context path must be a safe relative repository path".to_owned()
    })?;
    evidence::require_repository_path(&result.manifest.path).map_err(|_| {
        "ContextBuild result manifest path must be a safe relative repository path".to_owned()
    })?;
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
    evidence::require_hash(&result.context.hash, &context_bytes, "ContextBuild context")?;
    evidence::require_hash(
        &result.manifest.hash,
        &manifest_bytes,
        "ContextBuild manifest",
    )?;
    let manifest: ContextManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid ContextBuild manifest: {error}"))?;
    let manifest_context_path = Path::new(&result.manifest.path)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(&manifest.context.path);
    if manifest.schema != "methexis.context-manifest/v1alpha1"
        || manifest.build_id != result.build_id
        || manifest.context.hash != result.context.hash
        || manifest.context.path != "context.md"
        || manifest_context_path != Path::new(&result.context.path)
        || manifest.plan.tokenizer_profile != TOKENIZER_PROFILE
    {
        return Err("ContextBuild result and manifest identities differ".to_owned());
    }
    Ok(ContextCapture {
        result,
        request,
        context: evidence::captured(context_path.to_string_lossy().into_owned(), context_bytes)?,
        manifest: evidence::captured(manifest_path.to_string_lossy().into_owned(), manifest_bytes)?,
        active_checkpoint: manifest.plan.checkpoint,
        included_ids: manifest
            .plan
            .units
            .into_iter()
            .map(|unit| unit.id)
            .collect(),
    })
}

#[cfg(test)]
pub(in crate::review_packet) fn capture_context_from_result(
    repository: &Path,
    request: crate::review_protocol::Captured,
    result: ContextResult,
) -> Result<ContextCapture, String> {
    capture_context_artifacts(repository, request, result)
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
