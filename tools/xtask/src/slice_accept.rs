use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use serde::Deserialize;

use crate::{
    bounded_file, git,
    impact::{self, ImpactInput},
    review_protocol, slice_close, slice_gate, slice_status, slice_worktree,
};

const MESSAGE_LIMIT: usize = 64 * 1024;
const REQUEST_LIMIT: usize = 64 * 1024;
const ACCEPT_REQUEST_SCHEMA: &str = "yo.slice-accept-request/v1alpha1";
const ACCEPT_RESULT_SCHEMA: &str = "yo.slice-accept-result/v1alpha1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptRequest {
    schema: String,
    slice: String,
    gate_request_path: String,
    gate_request_hash: String,
    message_source_path: String,
    message_source_hash: String,
    message_output_path: String,
    close_prepare_request_path: String,
    close_prepare_request_hash: String,
    close_plan_path: String,
    push: Push,
    approval_scope: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Push {
    remote: String,
    reference: String,
}

pub(crate) fn accept(repository: &Path, request_path: &Path) -> Result<(), String> {
    let request_bytes =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice accept request")?;
    let request: AcceptRequest = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid Slice accept request {}: {error}",
            request_path.display()
        )
    })?;
    validate_accept_request(&request)?;

    let state = slice_status::locate(repository, &request.slice)?;
    if !state.clean {
        return Err("Slice worktree must be clean before accepted integration".to_owned());
    }
    let integration = integration_worktree(repository, &state.bound.base_ref)?;
    slice_worktree::ensure_clean(&integration, "integration worktree", "Slice acceptance")?;
    let integration_ref = slice_worktree::current_branch_ref(&integration, "Slice acceptance")?;
    if integration_ref != state.bound.base_ref || request.push.reference != integration_ref {
        return Err(format!(
            "Slice acceptance must push its bound integration ref `{}`",
            state.bound.base_ref
        ));
    }
    let integration_head = slice_worktree::resolve_commit(&integration, &integration_ref)?;
    if !git::trusted_succeeds_in(
        &integration,
        &[
            "merge-base",
            "--is-ancestor",
            &state.bound.base,
            &integration_head,
        ],
    )? {
        return Err("bound Slice base is not an ancestor of current integration HEAD".to_owned());
    }

    let workspace = slice_worktree::workspace_root(repository)?;
    let gate_path = resolve(&workspace, &request.gate_request_path);
    let message_source = resolve(&workspace, &request.message_source_path);
    let message_output = resolve(&workspace, &request.message_output_path);
    let close_prepare = resolve(&workspace, &request.close_prepare_request_path);
    let close_plan = resolve(&workspace, &request.close_plan_path);
    require_hash(&gate_path, &request.gate_request_hash, "Slice gate request")?;
    require_hash(
        &message_source,
        &request.message_source_hash,
        "accepted commit message source",
    )?;
    require_hash(
        &close_prepare,
        &request.close_prepare_request_hash,
        "Slice close preparation request",
    )?;
    let gate = slice_gate::ready(&state.worktree, &gate_path)?;
    if gate.slice != request.slice || gate.candidate_commit != state.head {
        return Err("ready gate does not identify the registered Slice HEAD".to_owned());
    }
    let expected_scope = effect_scope(
        &request.slice,
        &state.head,
        &request.push.remote,
        &integration_ref,
    );
    if request.approval_scope != expected_scope {
        return Err(format!(
            "Slice accept approval_scope must equal `{expected_scope}`"
        ));
    }
    require_gate_scope(&gate_path, &expected_scope)?;

    revalidate_inputs(
        &request,
        request_path,
        &request_bytes,
        &gate_path,
        &message_source,
        &close_prepare,
    )?;
    prepare_commit_message(
        &state.worktree,
        &gate_path,
        &message_source,
        &message_output,
    )?;
    revalidate_inputs(
        &request,
        request_path,
        &request_bytes,
        &gate_path,
        &message_source,
        &close_prepare,
    )?;
    slice_worktree::ensure_clean(&integration, "integration worktree", "Slice acceptance")?;
    slice_worktree::ensure_clean(&state.worktree, "Slice worktree", "Slice acceptance")?;
    slice_worktree::expect_ref(&integration, &integration_ref, &integration_head)?;
    slice_worktree::expect_ref(&state.worktree, &state.branch, &state.head)?;

    let accepted_commit = integrate_candidate(
        &integration,
        &integration_ref,
        &integration_head,
        &state.worktree,
        &state.branch,
        &state.bound.base,
        &state.head,
        &message_output,
    )?;

    let push_spec = format!("{integration_ref}:{integration_ref}");
    let push_status = git::command_in(&integration, false)
        .args(["push", "--porcelain", &request.push.remote, &push_spec])
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("cannot start accepted integration push: {error}"))?;
    if !push_status.success() {
        return Err(format!(
            "accepted commit {accepted_commit} exists locally but push failed ({push_status})"
        ));
    }

    revalidate_inputs(
        &request,
        request_path,
        &request_bytes,
        &gate_path,
        &message_source,
        &close_prepare,
    )?;
    slice_close::prepare_metrics(&integration, &close_prepare)?;
    slice_close::plan(&integration, &request.slice, Some(&close_plan))?;
    slice_close::apply(&integration, &close_plan)?;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema": ACCEPT_RESULT_SCHEMA,
            "ok": true,
            "status": "accepted",
            "slice": request.slice,
            "candidate_commit": state.head,
            "accepted_commit": accepted_commit,
            "integration_ref": integration_ref,
            "remote": request.push.remote,
            "pushed": true,
            "closed": true
        }))
        .map_err(|error| format!("cannot encode Slice accept result: {error}"))?
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn integrate_candidate(
    integration: &Path,
    integration_ref: &str,
    integration_head: &str,
    candidate_repository: &Path,
    candidate_branch: &str,
    candidate_base: &str,
    candidate_head: &str,
    message_output: &Path,
) -> Result<String, String> {
    integrate_candidate_with(
        integration,
        integration_ref,
        integration_head,
        candidate_repository,
        candidate_branch,
        candidate_base,
        candidate_head,
        message_output,
        impact::review_coverage::create_accepted_commit,
    )
}

#[allow(clippy::too_many_arguments)]
fn integrate_candidate_with(
    integration: &Path,
    integration_ref: &str,
    integration_head: &str,
    candidate_repository: &Path,
    candidate_branch: &str,
    candidate_base: &str,
    candidate_head: &str,
    message_output: &Path,
    commit: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<String, String> {
    let candidate_diff = canonical_diff(candidate_repository, candidate_base, candidate_head)?;
    let changed_paths =
        candidate_changed_paths(candidate_repository, candidate_base, candidate_head)?;
    let branch = integration_ref
        .strip_prefix("refs/heads/")
        .ok_or_else(|| format!("unsupported integration ref `{integration_ref}`"))?;
    let preflight = ImpactInput {
        message: git::read(message_output, "prepared accepted commit message")?,
        changed_paths: changed_paths.clone(),
        branch: branch.to_owned(),
        merge_head: None,
        repository: integration.to_path_buf(),
        inherit_git_environment: false,
    };
    impact::preflight::check_candidate(&preflight, &candidate_diff)?;
    slice_worktree::ensure_clean(integration, "integration worktree", "Slice acceptance")?;
    slice_worktree::expect_ref(integration, integration_ref, integration_head)?;
    slice_worktree::expect_ref(candidate_repository, candidate_branch, candidate_head)?;

    let merge_status = git::command_in(integration, false)
        .args(["merge", "--squash", candidate_branch])
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("cannot start accepted Slice squash: {error}"))?;
    if !merge_status.success() {
        return Err(format!(
            "accepted Slice squash failed ({merge_status}); integration worktree requires inspection"
        ));
    }
    let staged_diff = git::output_bytes_in(
        integration,
        &[
            "diff",
            "--cached",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            "--",
        ],
        false,
    )?;
    if staged_diff != candidate_diff {
        return Err(
            "squashed index differs from the exact reviewed candidate diff; integration worktree requires inspection"
                .to_owned(),
        );
    }

    let commit_result = (|| {
        let input = ImpactInput::load_from(
            integration,
            message_output.to_path_buf(),
            None,
            Some(branch.to_owned()),
            true,
        )?;
        impact::preflight::check(&input)?;
        commit(&input.repository, message_output)
    })();
    if let Err(error) = commit_result {
        return Err(recover_precommit_failure(
            integration,
            integration_ref,
            integration_head,
            &candidate_diff,
            &changed_paths,
            error,
        ));
    }
    let accepted_commit = slice_worktree::resolve_commit(integration, integration_ref)?;
    let accepted_diff = canonical_diff(
        integration,
        &format!("{accepted_commit}^"),
        &accepted_commit,
    )?;
    if accepted_diff != candidate_diff {
        return Err("accepted commit differs from the exact reviewed candidate diff".to_owned());
    }
    Ok(accepted_commit)
}

fn validate_accept_request(request: &AcceptRequest) -> Result<(), String> {
    if request.schema != ACCEPT_REQUEST_SCHEMA {
        return Err(format!(
            "unsupported Slice accept request schema `{}`; expected `{ACCEPT_REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    for (value, label) in [
        (&request.gate_request_path, "gate_request_path"),
        (&request.message_source_path, "message_source_path"),
        (&request.message_output_path, "message_output_path"),
        (
            &request.close_prepare_request_path,
            "close_prepare_request_path",
        ),
        (&request.close_plan_path, "close_plan_path"),
    ] {
        if value.is_empty() || value.len() > 4096 || value.contains('\0') {
            return Err(format!("{label} must be a non-empty bounded path"));
        }
    }
    for (value, label) in [
        (&request.gate_request_hash, "gate_request_hash"),
        (&request.message_source_hash, "message_source_hash"),
        (
            &request.close_prepare_request_hash,
            "close_prepare_request_hash",
        ),
    ] {
        if value.len() != 71
            || !value.starts_with("sha256:")
            || !value[7..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(format!("{label} must be sha256:<64 lowercase hex>"));
        }
    }
    if request.push.remote.is_empty()
        || request.push.remote.len() > 64
        || !request
            .push
            .remote
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("push remote must be one bounded Git remote token".to_owned());
    }
    Ok(())
}

fn effect_scope(slice: &str, candidate: &str, remote: &str, reference: &str) -> String {
    format!(
        "yo.slice-accept-effects/v1alpha1;slice={slice};candidate={candidate};squash=true;push={remote}:{reference};close=true"
    )
}

fn require_gate_scope(path: &Path, expected: &str) -> Result<(), String> {
    let bytes = bounded_file::read_regular(path, REQUEST_LIMIT, "Slice gate request")?;
    let gate: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Slice gate request: {error}"))?;
    let kind = gate
        .pointer("/approval/kind")
        .and_then(serde_json::Value::as_str);
    let scope = gate
        .pointer("/approval/scope")
        .and_then(serde_json::Value::as_str);
    if kind != Some("exact_candidate") || scope != Some(expected) {
        return Err(
            "one-command acceptance requires the ready gate's exact_candidate approval to name the canonical squash, push, and close effects"
                .to_owned(),
        );
    }
    Ok(())
}

fn revalidate_inputs(
    request: &AcceptRequest,
    request_path: &Path,
    request_bytes: &[u8],
    gate: &Path,
    message: &Path,
    close: &Path,
) -> Result<(), String> {
    if bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice accept request")?
        != request_bytes
    {
        return Err("Slice accept request changed before integration".to_owned());
    }
    require_hash(gate, &request.gate_request_hash, "Slice gate request")?;
    require_hash(
        message,
        &request.message_source_hash,
        "accepted commit message source",
    )?;
    require_hash(
        close,
        &request.close_prepare_request_hash,
        "Slice close preparation request",
    )
}

fn require_hash(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    let bytes = bounded_file::read_regular(path, REQUEST_LIMIT, label)?;
    if review_protocol::digest(&bytes) == expected {
        Ok(())
    } else {
        Err(format!(
            "{label} hash differs from the Slice accept request"
        ))
    }
}

fn resolve(workspace: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    }
}

fn integration_worktree(repository: &Path, reference: &str) -> Result<PathBuf, String> {
    let matches = slice_worktree::worktrees(repository)?
        .into_iter()
        .filter(|worktree| worktree.branch.as_deref() == Some(reference))
        .map(|worktree| worktree.path)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(format!(
            "no registered integration worktree has branch `{reference}`"
        )),
        _ => Err(format!(
            "multiple registered integration worktrees have branch `{reference}`"
        )),
    }
}

fn candidate_changed_paths(
    repository: &Path,
    base: &str,
    candidate: &str,
) -> Result<Vec<String>, String> {
    let output = git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--name-only",
            "-z",
            "--diff-filter=ACDMR",
            "--no-renames",
            base,
            candidate,
            "--",
        ],
    )?;
    let paths = output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path)
                .map(str::to_owned)
                .map_err(|error| format!("candidate changed path must be UTF-8: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Err("accepted candidate has no changed paths".to_owned());
    }
    Ok(paths)
}

fn recover_precommit_failure(
    integration: &Path,
    integration_ref: &str,
    integration_head: &str,
    candidate_diff: &[u8],
    changed_paths: &[String],
    error: String,
) -> String {
    match restore_exact_squash(
        integration,
        integration_ref,
        integration_head,
        candidate_diff,
        changed_paths,
    ) {
        Ok(()) => format!(
            "{error}; the exact staged squash was automatically restored and the integration worktree is clean"
        ),
        Err(restore) => format!(
            "{error}; automatic pre-commit restoration was not safe: {restore}; inspect the integration worktree"
        ),
    }
}

fn restore_exact_squash(
    integration: &Path,
    integration_ref: &str,
    integration_head: &str,
    candidate_diff: &[u8],
    changed_paths: &[String],
) -> Result<(), String> {
    slice_worktree::expect_ref(integration, integration_ref, integration_head)?;
    let staged = git::output_bytes_in(
        integration,
        &[
            "diff",
            "--cached",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            "--",
        ],
        false,
    )?;
    if staged != candidate_diff {
        return Err("staged bytes no longer equal the exact candidate diff".to_owned());
    }
    let status = git::command_in(integration, false)
        .args(["restore", "--source=HEAD", "--staged", "--worktree", "--"])
        .args(changed_paths)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("cannot start exact staged-squash restoration: {error}"))?;
    if !status.success() {
        return Err(format!("exact staged-squash restoration failed ({status})"));
    }
    slice_worktree::ensure_clean(
        integration,
        "integration worktree",
        "Slice acceptance rollback",
    )
}

fn canonical_diff(repository: &Path, base: &str, candidate: &str) -> Result<Vec<u8>, String> {
    git::trusted_output_bytes_in(
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

pub(crate) fn prepare_commit_message(
    repository: &Path,
    gate_request: &Path,
    message_source: &Path,
    output: &Path,
) -> Result<(), String> {
    let gate = slice_gate::ready(repository, gate_request)?;
    let source = bounded_file::read_regular(
        message_source,
        MESSAGE_LIMIT,
        "accepted commit message source",
    )?;
    let message = compose_message(&source, &gate.commit_trailers)?;

    let current_gate = slice_gate::ready(repository, gate_request)?;
    let current_source = bounded_file::read_regular(
        message_source,
        MESSAGE_LIMIT,
        "accepted commit message source",
    )?;
    if current_gate != gate || current_source != source {
        return Err("post-gate commit inputs changed before publication".to_owned());
    }

    let created = bounded_file::publish_new_or_exact(
        output,
        &message,
        MESSAGE_LIMIT,
        "prepared accepted commit message",
    )?;
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema": "yo.slice-commit-message-publication/v1alpha1",
            "ok": true,
            "status": if created { "written" } else { "reused" },
            "slice": gate.slice,
            "candidate_commit": gate.candidate_commit,
            "diff_hash": gate.diff_hash,
            "message_path": output,
            "message_hash": review_protocol::digest(&message)
        }))
        .map_err(|error| format!("cannot encode commit message publication: {error}"))?
    );
    Ok(())
}

fn compose_message(source: &[u8], trailers: &[String]) -> Result<Vec<u8>, String> {
    let source = std::str::from_utf8(source)
        .map_err(|error| format!("accepted commit message source must be UTF-8: {error}"))?;
    if source.contains('\0') || source.contains('\r') {
        return Err("accepted commit message source must use LF text without NUL bytes".to_owned());
    }
    let source = source.trim_end_matches('\n');
    if source.trim().is_empty() {
        return Err("accepted commit message source must not be blank".to_owned());
    }
    if source
        .lines()
        .any(|line| line.starts_with("Slice-Review:") || line.starts_with("Review-Coverage:"))
    {
        return Err(
            "accepted commit message source must omit gate-derived review trailers".to_owned(),
        );
    }
    if trailers.is_empty() {
        return Err("ready gate returned no commit trailers".to_owned());
    }
    let mut message = String::with_capacity(
        source.len() + trailers.iter().map(String::len).sum::<usize>() + trailers.len() + 3,
    );
    message.push_str(source);
    message.push_str("\n\n");
    message.push_str(&trailers.join("\n"));
    message.push('\n');
    if message.len() > MESSAGE_LIMIT {
        return Err(format!(
            "prepared accepted commit message exceeds the {MESSAGE_LIMIT}-byte limit"
        ));
    }
    Ok(message.into_bytes())
}

#[cfg(test)]
#[path = "slice_accept/tests.rs"]
mod tests;
