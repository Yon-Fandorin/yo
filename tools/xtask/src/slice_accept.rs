use std::{path::Path, process::Stdio};

use crate::{bounded_file, git, slice_close, slice_gate, slice_status, slice_worktree};

mod integrate;
mod message;
mod prepare;
mod request;
mod rollback;

use self::{
    integrate::{integrate_candidate, integration_worktree},
    message::{MESSAGE_LIMIT, compose_message},
    request::{
        ACCEPT_REQUEST_SCHEMA, ACCEPT_REQUEST_SCHEMA_V1_ALPHA2, ACCEPT_REQUEST_SCHEMA_V1_ALPHA3,
        AcceptRequest, COMMIT_CANDIDATE_HK_RECEIPT, COMMIT_GIT_HOOKS, Push, REQUEST_LIMIT,
        effect_scope, fast_commit_verification, fast_effect_scope, require_commit_verification,
        require_gate_authorization, require_hash, resolve, revalidate_inputs,
        validate_accept_request,
    },
};

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
    if integration_ref != state.bound.base_ref {
        return Err(format!(
            "Slice acceptance must use its bound integration ref `{}`",
            state.bound.base_ref
        ));
    }
    if request
        .push
        .as_ref()
        .is_some_and(|push| push.reference != integration_ref)
    {
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
    let expected_scope = request.expected_effect_scope(&state.head, &integration_ref)?;
    let recorded_scope = request.effect_scope()?;
    if recorded_scope != expected_scope {
        return Err(format!(
            "Slice accept effect scope must equal `{expected_scope}`"
        ));
    }
    require_gate_authorization(&gate_path, &expected_scope, &request.schema)?;

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

    let current_gate = slice_gate::ready(&state.worktree, &gate_path)?;
    if current_gate != gate {
        return Err("ready gate or validation context changed before accepted commit".to_owned());
    }
    let commit_verification = request.commit_verification()?;
    require_commit_verification(
        commit_verification,
        &current_gate,
        &state.bound.base,
        &integration_head,
    )?;
    let accepted_commit = integrate_candidate(
        &integration,
        &integration_ref,
        &integration_head,
        &state.worktree,
        &state.branch,
        &state.bound.base,
        &state.head,
        &message_output,
        commit_verification,
    )?;

    if let Some(push) = &request.push {
        let push_spec = format!("{integration_ref}:{integration_ref}");
        let push_status = git::command_in(&integration, false)
            .args(["push", "--porcelain", &push.remote, &push_spec])
            .stdin(Stdio::null())
            .status()
            .map_err(|error| format!("cannot start accepted integration push: {error}"))?;
        if !push_status.success() {
            return Err(format!(
                "accepted commit {accepted_commit} exists locally but push failed ({push_status})"
            ));
        }
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
    let result = accept_result(
        &request,
        &state.head,
        &accepted_commit,
        &integration_ref,
        commit_verification,
    );
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode Slice accept result: {error}"))?
    );
    Ok(())
}

fn accept_result(
    request: &AcceptRequest,
    candidate_commit: &str,
    accepted_commit: &str,
    integration_ref: &str,
    commit_verification: &str,
) -> serde_json::Value {
    let mut result = serde_json::json!({
        "schema": request.result_schema(),
        "ok": true,
        "status": "accepted",
        "slice": request.slice,
        "candidate_commit": candidate_commit,
        "accepted_commit": accepted_commit,
        "integration_ref": integration_ref,
        "remote": request.push.as_ref().map(|push| push.remote.as_str()),
        "pushed": request.push.is_some(),
        "closed": true
    });
    if request.schema == ACCEPT_REQUEST_SCHEMA_V1_ALPHA3 {
        result
            .as_object_mut()
            .expect("Slice accept result is an object")
            .insert(
                "commit_verification".to_owned(),
                serde_json::Value::String(commit_verification.to_owned()),
            );
    }
    result
}

pub(crate) use message::prepare_commit_message;
pub(crate) use prepare::prepare;

#[cfg(test)]
#[path = "slice_accept/tests.rs"]
mod tests;
