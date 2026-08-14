use std::{io, path::Path};

use super::{
    MAX_REQUEST_BYTES, PreparedReadiness,
    capture::{
        capture_authorities, capture_context_request, capture_validation, same_capture,
        same_captures, same_named_captures,
    },
    model::{READINESS_RESULT_SCHEMA, READINESS_RESULT_SCHEMA_V1_ALPHA3, ReadinessResultRecord},
    prepare_readiness, require_exact_slice_branch,
    trusted_git::{trusted_ensure_clean, trusted_resolve_commit},
    write_result_to,
};
use crate::{
    bounded_file,
    review_protocol::{Captured, artifact, digest, resolve_input_path},
    slice_contract,
};

pub(super) fn run(
    repository: &Path,
    request_path: &Path,
    output: &mut impl io::Write,
) -> Result<(), String> {
    let prepared = prepare_readiness(
        repository,
        request_path,
        "checking review request readiness",
    )?;
    run_test_hook()?;
    final_revalidate(&prepared)?;

    write_result_to(
        output,
        &ReadinessResultRecord {
            schema: if prepared.activation_request.is_some() {
                READINESS_RESULT_SCHEMA_V1_ALPHA3
            } else {
                READINESS_RESULT_SCHEMA
            },
            ok: true,
            operation: "check_slice_review_request_readiness",
            status: "input_boundaries_ready",
            artifacts_published: false,
            authority: prepared.activation_request.as_ref().map(|_| "prospective"),
            slice: prepared.slice.clone(),
            base_commit: prepared.base_commit.clone(),
            trusted_commit: prepared.trusted_commit.clone(),
            candidate_commit: prepared.candidate_commit.clone(),
            request: artifact(&prepared.request_capture),
            slice_contract: artifact(&prepared.slice_contract),
            context_request: artifact(&prepared.context_request),
            activation_request: prepared.activation_request.as_ref().map(artifact),
            required_knowledge_id_count: prepared.required_knowledge_ids.len(),
            repository_authority_count: prepared.authorities.len(),
            validation_evidence_count: prepared.validation.len(),
            external_operation_evidence_count: prepared
                .validation
                .iter()
                .filter(|evidence| super::external_operation::is_evidence_name(&evidence.name))
                .count(),
            review_lens_count: prepared.lenses.len(),
            review_question_count: prepared.request.review_questions.len(),
        },
        "review request readiness result",
    )
}

fn final_revalidate(prepared: &PreparedReadiness) -> Result<(), String> {
    let repository = &prepared.repository;
    trusted_ensure_clean(
        repository,
        "candidate worktree",
        "returning review request readiness",
    )?;
    if trusted_resolve_commit(repository, "HEAD")? != prepared.candidate_commit {
        return Err("candidate HEAD changed during review request readiness".to_owned());
    }
    if trusted_resolve_commit(repository, "refs/heads/develop")? != prepared.trusted_commit {
        return Err("trusted integration changed during review request readiness".to_owned());
    }
    require_current_file(
        Path::new(&prepared.request_capture.path),
        &prepared.request_capture,
        "Slice review packet request",
    )?;

    let bound = slice_contract::trusted_bound_slice(repository)?;
    require_current_file(
        Path::new(&prepared.slice_contract.path),
        &prepared.slice_contract,
        "Slice contract",
    )?;
    if bound.slice != prepared.slice
        || bound.base != prepared.base_commit
        || bound.contract_id != prepared.slice_contract.hash
        || bound.contract_path != Path::new(&prepared.slice_contract.path)
    {
        return Err("bound Slice contract identity changed during readiness check".to_owned());
    }

    let context = capture_context_request(
        repository,
        &resolve_input_path(repository, &prepared.request.context_request_path),
    )?;
    if !same_capture(&context, &prepared.context_request) {
        return Err("ContextBuild request changed during readiness check".to_owned());
    }
    if let (Some(path), Some(expected)) = (
        prepared.request.activation_request_path.as_deref(),
        prepared.activation_request.as_ref(),
    ) {
        let current = capture_context_request(repository, &resolve_input_path(repository, path))?;
        if !same_capture(&current, expected) {
            return Err("activation request changed during readiness check".to_owned());
        }
    }
    let authorities = capture_authorities(
        repository,
        &prepared.candidate_commit,
        &prepared.request.repository_authority_paths,
    )?;
    if !same_captures(&authorities, &prepared.authorities) {
        return Err("repository authority inputs changed during readiness check".to_owned());
    }
    let validation = capture_validation(
        repository,
        &prepared.candidate_commit,
        &prepared.request.validation_evidence,
    )?;
    if !same_named_captures(&validation, &prepared.validation) {
        return Err("validation evidence changed during readiness check".to_owned());
    }
    slice_contract::trusted_check_bound_scope(repository)?;
    require_exact_slice_branch(repository, &bound)?;
    Ok(())
}

fn require_current_file(path: &Path, expected: &Captured, label: &str) -> Result<(), String> {
    let bytes = bounded_file::read_regular(path, MAX_REQUEST_BYTES, label)?;
    if digest(&bytes) == expected.hash && bytes == expected.bytes {
        Ok(())
    } else {
        Err(format!("{label} changed during readiness check"))
    }
}

#[cfg(test)]
type TestHook = Box<dyn FnOnce() -> Result<(), String>>;

#[cfg(test)]
thread_local! {
    static TEST_HOOK: std::cell::RefCell<Option<TestHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn set_test_hook(hook: impl FnOnce() -> Result<(), String> + 'static) {
    TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(test)]
fn run_test_hook() -> Result<(), String> {
    let hook = TEST_HOOK.with(|slot| slot.borrow_mut().take());
    hook.map_or(Ok(()), |hook| hook())
}

#[cfg(not(test))]
fn run_test_hook() -> Result<(), String> {
    Ok(())
}
