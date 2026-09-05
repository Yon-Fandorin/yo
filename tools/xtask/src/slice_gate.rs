use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use model::{
    Approval, ApprovalResult, REQUEST_SCHEMA, RESULT_SCHEMA, Request, ResultDocument, ReviewResult,
    Risk, ValidationResult,
};

use crate::{
    bounded_file, git,
    impact::{
        review_coverage,
        slice_review::{self, Lens},
    },
    review_protocol, slice_contract, validation_summary,
};

mod model;
mod prepare;

pub(crate) use prepare::run as prepare_request;

#[cfg(test)]
mod tests;

const REQUEST_LIMIT: usize = 64 * 1024;
const EVIDENCE_LIMIT: usize = 64 * 1024;
const MAX_PATHS: usize = 256;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_EVIDENCE: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadyGate {
    pub(crate) slice: String,
    pub(crate) candidate_commit: String,
    pub(crate) diff_hash: String,
    pub(crate) validation: Vec<ReadyValidation>,
    pub(crate) review_count: usize,
    pub(crate) known_unverified_environments: Vec<String>,
    pub(crate) commit_trailers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadyValidation {
    pub(crate) name: String,
    pub(crate) argv: Vec<String>,
    pub(crate) status: String,
    pub(crate) reused: bool,
    pub(crate) current_reusable_context: bool,
}

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let output = evaluate(repository, request_path)?;
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|error| format!("cannot encode Slice gate result: {error}"))?
    );
    Ok(())
}

pub(crate) fn ready(repository: &Path, request_path: &Path) -> Result<ReadyGate, String> {
    let result = evaluate(repository, request_path)?;
    if result.next_action != "integrate" {
        return Err(format!(
            "post-gate preparation requires next_action `integrate`, found `{}`",
            result.next_action
        ));
    }

    let request_bytes =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice gate request")?;
    if review_protocol::digest(&request_bytes) != result.request_hash {
        return Err("Slice gate request changed after ready evaluation".to_owned());
    }
    let request: Request = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid Slice gate request {} after ready evaluation: {error}",
            request_path.display()
        )
    })?;
    let mut commands = request
        .validation_evidence
        .into_iter()
        .map(|entry| {
            let bytes = captured(
                repository,
                &entry.result_path,
                &entry.result_hash,
                "validation result",
            )?;
            let current_context =
                validation_summary::current_reusable_context(&bytes).map_err(|error| {
                    format!(
                        "cannot revalidate current context for `{}`: {error}",
                        entry.name
                    )
                })?;
            Ok((entry.name, (entry.argv, current_context)))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let validation = result
        .validation
        .into_iter()
        .map(|entry| {
            let (argv, current_reusable_context) =
                commands.remove(&entry.name).ok_or_else(|| {
                    format!(
                        "ready gate validation `{}` lost its source command",
                        entry.name
                    )
                })?;
            Ok(ReadyValidation {
                name: entry.name,
                argv,
                status: entry.status,
                reused: entry.reused,
                current_reusable_context,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !commands.is_empty() {
        return Err("ready gate result omitted a source validation command".to_owned());
    }

    Ok(ReadyGate {
        slice: result.slice,
        candidate_commit: result.candidate_commit,
        diff_hash: result.diff_hash,
        validation,
        review_count: result.review.len(),
        known_unverified_environments: result.known_unverified_environments,
        commit_trailers: result.commit_trailers,
    })
}

fn evaluate(repository: &Path, request_path: &Path) -> Result<ResultDocument, String> {
    let request_bytes =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice gate request")?;
    let request: Request = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid Slice gate request {}: {error}",
            request_path.display()
        )
    })?;
    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported Slice gate request schema `{}`; expected `{REQUEST_SCHEMA}`",
            request.schema
        ));
    }
    validate_request_bounds(&request)?;

    let bound = slice_contract::trusted_bound_slice(repository)?;
    slice_contract::trusted_check_bound_scope(repository)?;
    require_clean(repository)?;

    let candidate = trusted_line(repository, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    review_protocol::require_commit(&request.candidate_commit, "candidate_commit")?;
    if request.candidate_commit != candidate {
        return Err(format!(
            "Slice gate request is stale: candidate {} does not match clean HEAD {candidate}",
            request.candidate_commit
        ));
    }

    let changed_paths = changed_paths(repository, &bound.base, &candidate)?;
    if changed_paths.is_empty() {
        return Err("Slice gate refuses a candidate with no base-to-HEAD changes".to_owned());
    }
    let diff = git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            &bound.base,
            &candidate,
            "--",
        ],
    )?;
    let diff_hash = review_protocol::digest(&diff);

    let required_lenses = required_lenses(
        &request.required_lenses,
        &changed_paths,
        bound.base_ref.starts_with("refs/heads/wave/"),
    )?;
    let validation = validation_results(repository, &request, &candidate)?;
    let review = review_results(
        repository,
        &request,
        &candidate,
        &diff_hash,
        &required_lenses,
    )?;
    let approval = approval_result(
        &request.risk,
        request.approval.as_ref(),
        &candidate,
        &diff_hash,
    )?;

    let validation_ready =
        !validation.is_empty() && validation.iter().all(|entry| entry.status == "passed");
    let reviewed = required_lenses
        .iter()
        .all(|lens| review.iter().any(|entry| entry.lens == lens.label()));
    let next_action = if !validation_ready {
        "validate"
    } else if !reviewed {
        "review"
    } else if !approval.satisfied {
        "approve"
    } else {
        "integrate"
    };
    let status = if next_action == "integrate" {
        "ready"
    } else {
        "incomplete"
    };
    let commit_trailers = trailers(&review, &diff_hash, required_lenses.is_empty());

    run_final_revalidate_hook()?;
    final_revalidate(
        repository,
        request_path,
        &request_bytes,
        &bound,
        &candidate,
        &diff_hash,
        &changed_paths,
        &request,
    )?;

    Ok(ResultDocument {
        schema: RESULT_SCHEMA,
        ok: true,
        status,
        slice: bound.slice,
        contract_id: bound.contract_id,
        request_hash: review_protocol::digest(&request_bytes),
        base_commit: bound.base,
        candidate_commit: candidate,
        diff_hash,
        changed_paths,
        required_lenses: required_lenses
            .into_iter()
            .map(|lens| lens.label().to_owned())
            .collect(),
        validation,
        review,
        known_unverified_environments: request.known_unverified_environments,
        risk: request.risk,
        approval,
        commit_trailers,
        next_action,
    })
}

fn validate_request_bounds(request: &Request) -> Result<(), String> {
    if request.validation_evidence.len() > MAX_EVIDENCE {
        return Err(format!(
            "validation_evidence exceeds the {MAX_EVIDENCE}-entry limit"
        ));
    }
    if request.review_evidence.len() > 3 {
        return Err("review_evidence exceeds the three-lens limit".to_owned());
    }
    if request.required_lenses.len() > 3 {
        return Err("required_lenses exceeds the three-lens limit".to_owned());
    }
    if request.known_unverified_environments.len() > 16 {
        return Err("known_unverified_environments exceeds the 16-entry limit".to_owned());
    }
    compact(&request.risk.rationale, 512, "risk rationale")?;
    for environment in &request.known_unverified_environments {
        compact(environment, 256, "unverified environment")?;
    }
    if request.risk.classification == "routine" && !request.known_unverified_environments.is_empty()
    {
        return Err("routine risk cannot retain known unverified environments".to_owned());
    }
    if !matches!(
        request.risk.classification.as_str(),
        "routine" | "human-attention"
    ) {
        return Err("risk classification must be routine or human-attention".to_owned());
    }
    Ok(())
}

fn required_lenses(
    requested: &[String],
    changed_paths: &[String],
    integration_required: bool,
) -> Result<BTreeSet<Lens>, String> {
    let mut lenses = BTreeSet::new();
    for value in requested {
        let lens =
            Lens::parse(value).ok_or_else(|| format!("unknown required review lens `{value}`"))?;
        if !lenses.insert(lens) {
            return Err(format!("duplicate required review lens `{value}`"));
        }
    }
    let minimum = slice_review::minimum_lenses(changed_paths, integration_required);
    let omitted = minimum
        .difference(&lenses)
        .map(|lens| lens.label())
        .collect::<Vec<_>>();
    if !omitted.is_empty() {
        return Err(format!(
            "required_lenses omits path-derived minimum lenses: {}",
            omitted.join(", ")
        ));
    }
    Ok(lenses)
}

fn validation_results(
    repository: &Path,
    request: &Request,
    candidate: &str,
) -> Result<Vec<ValidationResult>, String> {
    let mut names = BTreeSet::new();
    let mut results = Vec::new();
    for entry in &request.validation_evidence {
        compact_id(&entry.name, 64, "validation name")?;
        if !names.insert(entry.name.as_str()) {
            return Err(format!("duplicate validation evidence `{}`", entry.name));
        }
        if entry.argv.is_empty() || entry.argv.len() > 32 {
            return Err(format!(
                "validation `{}` argv must contain 1..=32 values",
                entry.name
            ));
        }
        let argv_bytes = entry.argv.iter().try_fold(0usize, |total, value| {
            compact(value, 4096, "validation argv value")?;
            total
                .checked_add(value.len())
                .ok_or_else(|| "validation argv size overflow".to_owned())
        })?;
        if argv_bytes > 8192 {
            return Err(format!(
                "validation `{}` argv exceeds 8192 bytes",
                entry.name
            ));
        }
        exact_candidate(&entry.candidate_commit, candidate, "validation evidence")?;
        let bytes = captured(
            repository,
            &entry.result_path,
            &entry.result_hash,
            "validation result",
        )?;
        let summary = validation_summary::verify(
            repository,
            &bytes,
            &entry.name,
            &entry.argv,
            candidate,
            entry.reused,
        )
        .map_err(|error| format!("invalid validation result for `{}`: {error}", entry.name))?;
        if let Some(log_path) = &summary.log_path {
            compact(log_path, 4096, "validation log path")?;
        }
        results.push(ValidationResult {
            name: entry.name.clone(),
            status: summary.status,
            reused: entry.reused,
            result_hash: entry.result_hash.clone(),
        });
    }
    results.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(results)
}

fn review_results(
    repository: &Path,
    request: &Request,
    candidate: &str,
    diff_hash: &str,
    required_lenses: &BTreeSet<Lens>,
) -> Result<Vec<ReviewResult>, String> {
    let mut lenses = BTreeMap::new();
    for entry in &request.review_evidence {
        let lens = Lens::parse(&entry.lens)
            .ok_or_else(|| format!("unknown review lens `{}`", entry.lens))?;
        if !required_lenses.contains(&lens) {
            return Err(format!(
                "review evidence includes undeclared lens `{}`",
                entry.lens
            ));
        }
        if lenses.contains_key(&lens) {
            return Err(format!("duplicate review evidence for `{}`", entry.lens));
        }
        if !matches!(entry.verdict.as_str(), "clear" | "resolved") {
            return Err(format!(
                "review `{}` verdict must be clear or resolved",
                entry.lens
            ));
        }
        exact_candidate(&entry.candidate_commit, candidate, "review evidence")?;
        if entry.diff_hash != diff_hash {
            return Err(format!(
                "review `{}` is stale: diff {} does not match {diff_hash}",
                entry.lens, entry.diff_hash
            ));
        }
        let route_reviewer = review_coverage::reviewer_for_route(&entry.route, lens)?;
        if route_reviewer != entry.reviewer {
            return Err(format!(
                "review `{}` route does not match reviewer",
                entry.lens
            ));
        }
        compact_id(&entry.reviewer, 256, "reviewer")?;
        let _ = captured(
            repository,
            &entry.result_path,
            &entry.result_hash,
            "review result",
        )?;
        lenses.insert(
            lens,
            ReviewResult {
                lens: entry.lens.clone(),
                reviewer: entry.reviewer.clone(),
                route: entry.route.clone(),
                verdict: entry.verdict.clone(),
                result_hash: entry.result_hash.clone(),
            },
        );
    }
    Ok(lenses.into_values().collect())
}

fn approval_result(
    risk: &Risk,
    approval: Option<&Approval>,
    candidate: &str,
    diff_hash: &str,
) -> Result<ApprovalResult, String> {
    let required = true;
    let Some(approval) = approval else {
        return Ok(ApprovalResult {
            required,
            satisfied: false,
            kind: None,
            authority: None,
            scope: None,
        });
    };
    compact_id(&approval.authority, 128, "approval authority")?;
    if review_coverage::human_reviewer_for_route(&approval.authority).as_deref()
        != Some(approval.authority.as_str())
    {
        return Err("approval authority must use exactly human/<identity>".to_owned());
    }
    compact(&approval.scope, 512, "approval scope")?;
    match approval.kind.as_str() {
        "exact_candidate" => {
            let approved_candidate = approval
                .candidate_commit
                .as_deref()
                .ok_or_else(|| "exact_candidate approval requires candidate_commit".to_owned())?;
            let approved_diff = approval
                .diff_hash
                .as_deref()
                .ok_or_else(|| "exact_candidate approval requires diff_hash".to_owned())?;
            exact_candidate(approved_candidate, candidate, "approval")?;
            if approved_diff != diff_hash {
                return Err(format!(
                    "approval diff {approved_diff} does not match {diff_hash}"
                ));
            }
        },
        "standing_routine" if risk.classification == "routine" => {
            if approval.candidate_commit.is_some() || approval.diff_hash.is_some() {
                return Err(
                    "standing_routine approval must omit candidate_commit and diff_hash".to_owned(),
                );
            }
        },
        "standing_routine" => {
            return Err("human-attention risk requires exact_candidate approval".to_owned());
        },
        _ => return Err("approval kind must be exact_candidate or standing_routine".to_owned()),
    }
    Ok(ApprovalResult {
        required,
        satisfied: true,
        kind: Some(approval.kind.clone()),
        authority: Some(approval.authority.clone()),
        scope: Some(approval.scope.clone()),
    })
}

fn trailers(review: &[ReviewResult], diff_hash: &str, no_lens_required: bool) -> Vec<String> {
    if no_lens_required {
        return vec![
            "Slice-Review: none - no path-based or planner-added review lens applies".to_owned(),
        ];
    }
    let mut trailers = Vec::new();
    for entry in review {
        trailers.push(format!(
            "Slice-Review: {} - completed - {} - {}",
            entry.lens, entry.reviewer, entry.verdict
        ));
    }
    for entry in review {
        trailers.push(format!(
            "Review-Coverage: {} - exact - {} - {diff_hash}",
            entry.lens, entry.route
        ));
    }
    trailers
}

fn require_clean(repository: &Path) -> Result<(), String> {
    let status = git::trusted_output_bytes_in(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err("Slice gate requires a clean candidate worktree".to_owned())
    }
}

#[allow(clippy::too_many_arguments)]
fn final_revalidate(
    repository: &Path,
    request_path: &Path,
    request_bytes: &[u8],
    bound: &slice_contract::BoundSlice,
    candidate: &str,
    diff_hash: &str,
    changed: &[String],
    request: &Request,
) -> Result<(), String> {
    let current_request =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice gate request")?;
    if current_request != request_bytes {
        return Err("Slice gate request changed during evaluation".to_owned());
    }
    if slice_contract::trusted_bound_slice(repository)? != *bound {
        return Err("bound Slice identity changed during gate evaluation".to_owned());
    }
    slice_contract::trusted_check_bound_scope(repository)?;
    require_clean(repository)?;
    if trusted_line(repository, &["rev-parse", "--verify", "HEAD^{commit}"])? != candidate {
        return Err("candidate HEAD changed during gate evaluation".to_owned());
    }
    let current_changed = changed_paths(repository, &bound.base, candidate)?;
    if current_changed != changed {
        return Err("candidate changed paths changed during gate evaluation".to_owned());
    }
    let current_diff = git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            &bound.base,
            candidate,
            "--",
        ],
    )?;
    if review_protocol::digest(&current_diff) != diff_hash {
        return Err("canonical candidate diff changed during gate evaluation".to_owned());
    }
    for entry in &request.validation_evidence {
        let _ = captured(
            repository,
            &entry.result_path,
            &entry.result_hash,
            "validation result",
        )?;
    }
    for entry in &request.review_evidence {
        let _ = captured(
            repository,
            &entry.result_path,
            &entry.result_hash,
            "review result",
        )?;
    }
    Ok(())
}

#[cfg(test)]
type FinalRevalidateTestHook = Box<dyn FnOnce() -> Result<(), String>>;

#[cfg(test)]
thread_local! {
    static FINAL_REVALIDATE_TEST_HOOK: std::cell::RefCell<Option<FinalRevalidateTestHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_final_revalidate_test_hook(hook: impl FnOnce() -> Result<(), String> + 'static) {
    FINAL_REVALIDATE_TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(test)]
fn run_final_revalidate_hook() -> Result<(), String> {
    let hook = FINAL_REVALIDATE_TEST_HOOK.with(|slot| slot.borrow_mut().take());
    hook.map_or(Ok(()), |hook| hook())
}

#[cfg(not(test))]
fn run_final_revalidate_hook() -> Result<(), String> {
    Ok(())
}

fn changed_paths(repository: &Path, base: &str, candidate: &str) -> Result<Vec<String>, String> {
    let bytes = git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            base,
            candidate,
            "--",
        ],
    )?;
    let mut paths = bytes
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| {
            String::from_utf8(value.to_vec())
                .map_err(|_| "Slice gate does not support non-UTF-8 changed paths".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    paths.dedup();
    if paths.len() > MAX_PATHS || paths.iter().map(String::len).sum::<usize>() > MAX_PATH_BYTES {
        return Err(format!(
            "changed paths exceed the bounded result limit ({MAX_PATHS} paths or {MAX_PATH_BYTES} bytes)"
        ));
    }
    Ok(paths)
}

fn captured(
    repository: &Path,
    value: &str,
    expected: &str,
    label: &str,
) -> Result<Vec<u8>, String> {
    canonical_sha256(expected, &format!("{label} hash"))?;
    compact(value, 4096, &format!("{label} path"))?;
    let path = resolve_path(repository, value);
    let bytes = bounded_file::read_regular(&path, EVIDENCE_LIMIT, label)?;
    let actual = review_protocol::digest(&bytes);
    if actual != expected {
        return Err(format!(
            "{label} {} hash changed: expected {expected}, found {actual}",
            path.display()
        ));
    }
    Ok(bytes)
}

fn resolve_path(repository: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository.join(path)
    }
}

fn exact_candidate(value: &str, candidate: &str, label: &str) -> Result<(), String> {
    review_protocol::require_commit(value, &format!("{label} candidate_commit"))?;
    if value == candidate {
        Ok(())
    } else {
        Err(format!(
            "{label} is stale: candidate {value} does not match {candidate}"
        ))
    }
}

fn canonical_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.strip_prefix("sha256:").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    }) {
        Ok(())
    } else {
        Err(format!("{label} must be canonical SHA-256"))
    }
}

fn compact(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err(format!(
            "{label} must contain 1..={max} bytes without control characters"
        ))
    } else {
        Ok(())
    }
}

fn compact_id(value: &str, max: usize, label: &str) -> Result<(), String> {
    compact(value, max, label)?;
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "._/+:-".contains(character))
    {
        Ok(())
    } else {
        Err(format!("{label} contains unsupported characters"))
    }
}

fn trusted_line(repository: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = git::trusted_output_in(repository, arguments)?;
    let value = output.trim();
    if value.is_empty() || output.lines().count() != 1 {
        Err(format!(
            "trusted Git {} returned an invalid line",
            arguments.join(" ")
        ))
    } else {
        Ok(value.to_owned())
    }
}
