use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Component, Path},
    process::ExitCode,
};

use serde::Deserialize;

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
    validation_summary,
};

pub(super) struct ContextCapture {
    pub(super) result: ContextResult,
    pub(super) request: Captured,
    pub(super) context: Captured,
    pub(super) manifest: Captured,
    pub(super) active_checkpoint: CheckpointIdentity,
    pub(super) included_ids: Vec<String>,
}

pub(super) struct ProspectiveCapture {
    pub(super) activation_request: Captured,
    pub(super) proposed_checkpoint: Captured,
    pub(super) proposed_active_record: Captured,
    pub(super) predecessor_active_record_hash: Option<String>,
}

pub(super) struct Inputs {
    pub(super) base_commit: String,
    pub(super) candidate_commit: String,
    pub(super) diff: Captured,
    pub(super) context: ContextCapture,
    pub(super) prospective: Option<ProspectiveCapture>,
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
        || result.checkpoint.is_some()
        || result.activation_request.is_some()
        || result.predecessor_active_record_hash.is_some()
        || result.proposed_active_record_hash.is_some()
    {
        return Err("Methexis returned a non-success ContextBuild result".to_owned());
    }
    capture_context_artifacts(repository, request, result)
}

pub(super) fn capture_prospective_context_with_request(
    repository: &Path,
    candidate_commit: &str,
    activation_request_path: &Path,
    activation_request: Captured,
    context_request_path: &Path,
    context_request: Captured,
) -> Result<(ContextCapture, ProspectiveCapture), String> {
    capture_prospective_context(
        repository,
        candidate_commit,
        activation_request_path,
        activation_request,
        context_request,
        || resolve_prospective_context(activation_request_path, context_request_path),
    )
}

#[cfg(test)]
pub(super) fn capture_prospective_context_from_result(
    repository: &Path,
    candidate_commit: &str,
    activation_request_path: &Path,
    activation_request: Captured,
    context_request: Captured,
    result: ContextResult,
) -> Result<(ContextCapture, ProspectiveCapture), String> {
    capture_prospective_context(
        repository,
        candidate_commit,
        activation_request_path,
        activation_request,
        context_request,
        || Ok(result),
    )
}

fn capture_prospective_context(
    repository: &Path,
    candidate_commit: &str,
    activation_request_path: &Path,
    activation_request: Captured,
    context_request: Captured,
    resolve: impl FnOnce() -> Result<ContextResult, String>,
) -> Result<(ContextCapture, ProspectiveCapture), String> {
    let activation = parse_activation_request(&activation_request.bytes)?;
    let checkpoint_path = format!(
        "methexis/checkpoints/{}.yaml",
        activation
            .checkpoint_id
            .strip_prefix("sha256:")
            .expect("validated activation CheckpointId")
    );
    let proposal = capture_authorities(
        repository,
        candidate_commit,
        &[
            checkpoint_path.clone(),
            "methexis/active-checkpoint.yaml".to_owned(),
        ],
    )?;
    let proposed_active_record = proposal
        .iter()
        .find(|capture| capture.path == "methexis/active-checkpoint.yaml")
        .cloned()
        .ok_or_else(|| "prospective active record capture is missing".to_owned())?;
    let proposed_checkpoint = proposal
        .iter()
        .find(|capture| capture.path == checkpoint_path)
        .cloned()
        .ok_or_else(|| "prospective Checkpoint capture is missing".to_owned())?;
    require_hash(
        &activation.checkpoint_hash,
        &proposed_checkpoint.bytes,
        "prospective Checkpoint",
    )?;
    let result = resolve()?;
    let context = capture_context_artifacts(repository, context_request, result)?;
    let result = &context.result;
    let checkpoint = result
        .checkpoint
        .as_ref()
        .ok_or_else(|| "prospective ContextBuild result omitted its Checkpoint".to_owned())?;
    let result_request = result.activation_request.as_ref().ok_or_else(|| {
        "prospective ContextBuild result omitted its activation request".to_owned()
    })?;
    if result.schema != "methexis.activation-review-context-result/v1alpha1"
        || !result.ok
        || result.operation != "resolve_activation_review_context"
        || result.authority != "prospective"
        || checkpoint != &context.active_checkpoint
        || checkpoint.id != activation.checkpoint_id
        || checkpoint.hash != activation.checkpoint_hash
        || checkpoint.authority_basis_commit != result.trusted_commit
        || result_request.hash != activation_request.hash
        || resolve_input_path(repository, &result_request.path) != activation_request_path
        || result.predecessor_active_record_hash != activation.replace_active_hash
        || result.proposed_active_record_hash.as_deref()
            != Some(proposed_active_record.hash.as_str())
    {
        return Err(
            "prospective ContextBuild result, activation proposal, and manifest identities differ"
                .to_owned(),
        );
    }
    Ok((
        context,
        ProspectiveCapture {
            activation_request,
            proposed_checkpoint,
            proposed_active_record,
            predecessor_active_record_hash: activation.replace_active_hash,
        },
    ))
}

fn capture_context_artifacts(
    repository: &Path,
    request: Captured,
    result: ContextResult,
) -> Result<ContextCapture, String> {
    require_repository_path(&result.context.path).map_err(|_| {
        "ContextBuild result context path must be a safe relative repository path".to_owned()
    })?;
    require_repository_path(&result.manifest.path).map_err(|_| {
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
    require_hash(&result.context.hash, &context_bytes, "ContextBuild context")?;
    require_hash(
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

#[cfg(test)]
pub(super) fn capture_context_from_result(
    repository: &Path,
    request: Captured,
    result: ContextResult,
) -> Result<ContextCapture, String> {
    capture_context_artifacts(repository, request, result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActivationRequest {
    pub(super) schema: String,
    pub(super) checkpoint_id: String,
    pub(super) checkpoint_hash: String,
    pub(super) replace_active_hash: Option<String>,
}

pub(super) fn parse_activation_request(bytes: &[u8]) -> Result<ActivationRequest, String> {
    let request = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid activation request: {error}"))?;
    validate_activation_request(&request)?;
    Ok(request)
}

fn validate_activation_request(request: &ActivationRequest) -> Result<(), String> {
    if request.schema == "methexis.activation-request/v1alpha1"
        && valid_hash(&request.checkpoint_id)
        && valid_hash(&request.checkpoint_hash)
        && request
            .replace_active_hash
            .as_ref()
            .is_none_or(|hash| valid_hash(hash))
    {
        Ok(())
    } else {
        Err("activation request schema or hashes are invalid".to_owned())
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
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

fn resolve_prospective_context(
    activation_request_path: &Path,
    context_request_path: &Path,
) -> Result<ContextResult, String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let arguments = [
        OsString::from("resolve-activation-review-context"),
        activation_request_path.as_os_str().to_owned(),
        context_request_path.as_os_str().to_owned(),
    ];
    let code = methexis::run(arguments, &mut stdout, &mut stderr)
        .map_err(|error| format!("cannot run prospective Methexis ContextBuild: {error}"))?;
    if code != ExitCode::SUCCESS {
        return Err(format!(
            "prospective Methexis ContextBuild failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    serde_json::from_slice(&stdout)
        .map_err(|error| format!("invalid prospective Methexis ContextBuild result: {error}"))
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
        let canonical = std::fs::canonicalize(&path).map_err(|error| {
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
