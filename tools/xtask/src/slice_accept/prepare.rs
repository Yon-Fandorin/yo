use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    ACCEPT_REQUEST_SCHEMA, AcceptRequest, Push, effect_scope, integration_worktree,
    require_gate_scope, validate_accept_request,
};
use crate::{
    bounded_file, git, review_protocol, slice_close, slice_contract, slice_gate, slice_worktree,
};

const PREPARE_REQUEST_SCHEMA: &str = "yo.slice-accept-prepare-request/v1alpha1";
const PREPARE_RESULT_SCHEMA: &str = "yo.slice-accept-prepare-result/v1alpha1";
const INPUT_LIMIT: usize = 64 * 1024;
const ACCEPT_FILE: &str = "accept.json";
const CLOSE_PREPARE_FILE: &str = "close-prepare.json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareRequest {
    schema: String,
    gate_request_path: String,
    message_source_path: String,
    close_observations: slice_close::CloseObservations,
    push_remote: String,
}

#[derive(Serialize)]
struct PrepareResult<'a> {
    schema: &'static str,
    ok: bool,
    status: &'a str,
    slice: &'a str,
    base_commit: &'a str,
    candidate_commit: &'a str,
    diff_hash: &'a str,
    validation_count: usize,
    review_evidence_count: usize,
    commit_trailer_count: usize,
    approval_scope: &'a str,
    accept_request_path: &'a Path,
    accept_request_hash: String,
    close_prepare_request_path: &'a Path,
    close_prepare_request_hash: String,
    message_output_path: &'a Path,
    close_plan_path: &'a Path,
}

pub(crate) fn prepare(repository: &Path, request_path: &Path) -> Result<(), String> {
    prepare_with(repository, request_path, || Ok(()))
}

fn prepare_with(
    repository: &Path,
    request_path: &Path,
    before_revalidate: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let request_bytes = bounded_file::read_regular(
        request_path,
        INPUT_LIMIT,
        "Slice accept preparation request",
    )?;
    let request: PrepareRequest = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid Slice accept preparation request {}: {error}",
            request_path.display()
        )
    })?;
    validate_request(&request)?;

    let bound = slice_contract::trusted_bound_slice(repository)?;
    slice_contract::trusted_check_bound_scope(repository)?;
    slice_worktree::ensure_clean(repository, "Slice worktree", "post-gate preparation")?;
    let candidate = slice_worktree::resolve_commit(repository, "HEAD")?;
    let workspace = slice_worktree::workspace_root(repository)?;
    let gate_path = review_protocol::resolve_input_path(&workspace, &request.gate_request_path);
    let message_source =
        review_protocol::resolve_input_path(&workspace, &request.message_source_path);
    let gate_bytes = bounded_file::read_regular(&gate_path, INPUT_LIMIT, "Slice gate request")?;
    let message_bytes = bounded_file::read_regular(
        &message_source,
        super::MESSAGE_LIMIT,
        "accepted commit message source",
    )?;
    let gate = slice_gate::ready(repository, &gate_path)?;
    if gate.slice != bound.slice || gate.candidate_commit != candidate {
        return Err("ready gate does not identify the bound Slice HEAD".to_owned());
    }
    super::compose_message(&message_bytes, &gate.commit_trailers)?;

    let integration = integration_worktree(repository, &bound.base_ref)?;
    slice_worktree::ensure_clean(
        &integration,
        "integration worktree",
        "post-gate preparation",
    )?;
    let integration_ref =
        slice_worktree::current_branch_ref(&integration, "post-gate preparation")?;
    if integration_ref != bound.base_ref {
        return Err(format!(
            "post-gate preparation requires the bound integration ref `{}`",
            bound.base_ref
        ));
    }
    let integration_head = slice_worktree::resolve_commit(&integration, &integration_ref)?;
    if !git::trusted_succeeds_in(
        &integration,
        &[
            "merge-base",
            "--is-ancestor",
            &bound.base,
            &integration_head,
        ],
    )? {
        return Err("bound Slice base is not an ancestor of current integration HEAD".to_owned());
    }

    let approval_scope = effect_scope(
        &bound.slice,
        &candidate,
        &request.push_remote,
        &integration_ref,
    );
    require_gate_scope(&gate_path, &approval_scope)?;

    let coordination = workspace
        .join(".local-exclude")
        .join("coordination")
        .join(&bound.slice);
    let accept_path = coordination.join(ACCEPT_FILE);
    let close_prepare_path = coordination.join(CLOSE_PREPARE_FILE);
    let (message_output_path, close_plan_path) = temporary_output_paths(&bound.slice, &candidate);
    ensure_distinct_paths(
        request_path,
        &gate_path,
        &message_source,
        &accept_path,
        &close_prepare_path,
        &message_output_path,
        &close_plan_path,
    )?;

    let gate_request_path = portable_path(&workspace, &gate_path)?;
    let message_source_path = portable_path(&workspace, &message_source)?;
    let close_prepare_request_path = portable_path(&workspace, &close_prepare_path)?;
    let close_prepare_bytes = slice_close::close_prepare_request_bytes(
        &bound.slice,
        &gate_request_path,
        &request.close_observations,
    )?;
    slice_close::validate_close_prepare_request(&close_prepare_bytes, &gate, &candidate)?;

    let accept_request = AcceptRequest {
        schema: ACCEPT_REQUEST_SCHEMA.to_owned(),
        slice: bound.slice.clone(),
        gate_request_path,
        gate_request_hash: review_protocol::digest(&gate_bytes),
        message_source_path,
        message_source_hash: review_protocol::digest(&message_bytes),
        message_output_path: path_text(&message_output_path)?,
        close_prepare_request_path,
        close_prepare_request_hash: review_protocol::digest(&close_prepare_bytes),
        close_plan_path: path_text(&close_plan_path)?,
        push: Push {
            remote: request.push_remote.clone(),
            reference: integration_ref.clone(),
        },
        approval_scope: approval_scope.clone(),
    };
    validate_accept_request(&accept_request)?;
    let mut accept_bytes = serde_json::to_vec_pretty(&accept_request)
        .map_err(|error| format!("cannot encode prepared Slice accept request: {error}"))?;
    accept_bytes.push(b'\n');

    preflight_publication(
        &close_prepare_path,
        &close_prepare_bytes,
        "Slice close preparation request",
    )?;
    preflight_publication(&accept_path, &accept_bytes, "Slice accept request")?;
    before_revalidate()?;
    revalidate_inputs(
        repository,
        request_path,
        &request_bytes,
        &gate_path,
        &gate_bytes,
        &message_source,
        &message_bytes,
        &gate,
        &bound,
        &candidate,
        &integration,
        &integration_ref,
        &integration_head,
        &approval_scope,
    )?;

    let close_created = bounded_file::publish_new_or_exact(
        &close_prepare_path,
        &close_prepare_bytes,
        INPUT_LIMIT,
        "Slice close preparation request",
    )?;
    let accept_created = bounded_file::publish_new_or_exact(
        &accept_path,
        &accept_bytes,
        INPUT_LIMIT,
        "Slice accept request",
    )?;
    let status = match (close_created, accept_created) {
        (false, false) => "reused",
        _ => "written",
    };
    let result = PrepareResult {
        schema: PREPARE_RESULT_SCHEMA,
        ok: true,
        status,
        slice: &bound.slice,
        base_commit: &bound.base,
        candidate_commit: &candidate,
        diff_hash: &gate.diff_hash,
        validation_count: gate.validation.len(),
        review_evidence_count: gate.review_count,
        commit_trailer_count: gate.commit_trailers.len(),
        approval_scope: &approval_scope,
        accept_request_path: &accept_path,
        accept_request_hash: review_protocol::digest(&accept_bytes),
        close_prepare_request_path: &close_prepare_path,
        close_prepare_request_hash: review_protocol::digest(&close_prepare_bytes),
        message_output_path: &message_output_path,
        close_plan_path: &close_plan_path,
    };
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode Slice accept preparation result: {error}"))?
    );
    Ok(())
}

fn validate_request(request: &PrepareRequest) -> Result<(), String> {
    if request.schema != PREPARE_REQUEST_SCHEMA {
        return Err(format!(
            "unsupported Slice accept preparation schema `{}`; expected `{PREPARE_REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    for (value, label) in [
        (&request.gate_request_path, "gate_request_path"),
        (&request.message_source_path, "message_source_path"),
    ] {
        if value.is_empty() || value.len() > 4096 || value.contains('\0') {
            return Err(format!("{label} must be a non-empty bounded path"));
        }
    }
    if request.push_remote.is_empty()
        || request.push_remote.len() > 64
        || !request
            .push_remote
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("push_remote must be one bounded Git remote token".to_owned());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ensure_distinct_paths(
    request: &Path,
    gate: &Path,
    message_source: &Path,
    accept: &Path,
    close_prepare: &Path,
    message_output: &Path,
    close_plan: &Path,
) -> Result<(), String> {
    let paths = [
        request,
        gate,
        message_source,
        accept,
        close_prepare,
        message_output,
        close_plan,
    ];
    for (index, left) in paths.iter().enumerate() {
        if paths[index + 1..].iter().any(|right| left == right) {
            return Err("post-gate input and output paths must be distinct".to_owned());
        }
    }
    Ok(())
}

fn temporary_output_paths(slice: &str, candidate: &str) -> (PathBuf, PathBuf) {
    let prefix = format!("yo-{slice}-{}", &candidate[..12]);
    let directory = std::env::temp_dir();
    (
        directory.join(format!("{prefix}-commit-message.txt")),
        directory.join(format!("{prefix}-close-plan.json")),
    )
}

fn portable_path(workspace: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(workspace)
        .map_or_else(|_| path_text(path), path_text)
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .filter(|value| !value.is_empty() && value.len() <= 4096 && !value.contains('\0'))
        .map(str::to_owned)
        .ok_or_else(|| format!("path {} must be bounded UTF-8", path.display()))
}

fn preflight_publication(path: &Path, expected: &[u8], label: &str) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_file() || metadata.nlink() != 1 => {
            Err(format!("{label} must be a singly linked regular file"))
        },
        Ok(_) => {
            let current = bounded_file::read_regular(path, INPUT_LIMIT, label)?;
            require_unchanged(&current, expected, &format!("existing {label}"))
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot inspect {label} {}: {error}",
            path.display()
        )),
    }
}

#[cfg(unix)]
trait MetadataLinks {
    fn nlink(&self) -> u64;
}

#[cfg(unix)]
impl MetadataLinks for std::fs::Metadata {
    fn nlink(&self) -> u64 {
        std::os::unix::fs::MetadataExt::nlink(self)
    }
}

#[cfg(not(unix))]
trait MetadataLinks {
    fn nlink(&self) -> u64;
}

#[cfg(not(unix))]
impl MetadataLinks for std::fs::Metadata {
    fn nlink(&self) -> u64 {
        1
    }
}

#[allow(clippy::too_many_arguments)]
fn revalidate_inputs(
    repository: &Path,
    request_path: &Path,
    request_bytes: &[u8],
    gate_path: &Path,
    gate_bytes: &[u8],
    message_path: &Path,
    message_bytes: &[u8],
    gate: &slice_gate::ReadyGate,
    bound: &slice_contract::BoundSlice,
    candidate: &str,
    integration: &Path,
    integration_ref: &str,
    integration_head: &str,
    approval_scope: &str,
) -> Result<(), String> {
    let current_request = bounded_file::read_regular(
        request_path,
        INPUT_LIMIT,
        "Slice accept preparation request",
    )?;
    require_unchanged(
        request_bytes,
        &current_request,
        "Slice accept preparation request",
    )?;
    let current_gate = bounded_file::read_regular(gate_path, INPUT_LIMIT, "Slice gate request")?;
    require_unchanged(gate_bytes, &current_gate, "Slice gate request")?;
    let current_message = bounded_file::read_regular(
        message_path,
        super::MESSAGE_LIMIT,
        "accepted commit message source",
    )?;
    require_unchanged(
        message_bytes,
        &current_message,
        "accepted commit message source",
    )?;
    if slice_contract::trusted_bound_slice(repository)? != *bound
        || slice_gate::ready(repository, gate_path)? != *gate
        || slice_worktree::resolve_commit(repository, "HEAD")? != candidate
    {
        return Err("post-gate Slice inputs changed before publication".to_owned());
    }
    slice_worktree::ensure_clean(repository, "Slice worktree", "post-gate publication")?;
    slice_worktree::ensure_clean(integration, "integration worktree", "post-gate publication")?;
    if slice_worktree::current_branch_ref(integration, "post-gate publication")? != integration_ref
        || slice_worktree::resolve_commit(integration, integration_ref)? != integration_head
    {
        return Err("integration ref changed before post-gate publication".to_owned());
    }
    require_gate_scope(gate_path, approval_scope)
}

fn require_unchanged(original: &[u8], current: &[u8], label: &str) -> Result<(), String> {
    if original == current {
        Ok(())
    } else {
        Err(format!("{label} changed before post-gate publication"))
    }
}

#[cfg(test)]
#[path = "tests/prepare.rs"]
mod tests;
