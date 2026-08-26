mod carry;
mod model;

#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use model::{
    PrepareRequest, PrepareResult, REQUEST_SCHEMA_V1_ALPHA1, RESULT_SCHEMA,
    RESULT_SCHEMA_V1_ALPHA1, ReviewSource, ValidationCommand,
};

use super::{
    approval_result, changed_paths, evaluate,
    model::{Approval, Request, ReviewEvidence, Risk, ValidationEvidence},
    require_clean, required_lenses, review_results, trusted_line, validate_request_bounds,
    validation_results,
};
use crate::{
    bounded_file, git,
    impact::{review_coverage, slice_review::Lens},
    review_delta, review_egress,
    review_packet::VerifiedReview,
    review_protocol::{digest, relative, resolve_input_path},
    slice_contract,
};

const PREPARE_REQUEST_LIMIT: usize = 64 * 1024;
const GATE_REQUEST_LIMIT: usize = 64 * 1024;
const MANIFEST_LIMIT: usize = 8 * 1024 * 1024;
const REVIEW_RESULT_LIMIT: usize = 64 * 1024;

type VerifyReview<'a> = dyn Fn(&Path, &Path, &str) -> Result<VerifiedReview, String> + 'a;
type VerifyCarry<'a> = dyn Fn(
        &Path,
        &VerifiedReview,
        &str,
        &model::CanonicalApprovalReviewCarry,
    ) -> Result<model::CanonicalApprovalReviewCarryResult, String>
    + 'a;

pub(crate) fn run(
    repository: &Path,
    prepare_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    let result = prepare_with(
        repository,
        prepare_path,
        output_path,
        &|repository, path, hash| {
            review_delta::verify_chain_head(repository, path, hash, &mut BTreeSet::new(), 0)
        },
        &carry::verify,
    )?;
    println!(
        "{}",
        serde_json::to_string(&result)
            .map_err(|error| format!("cannot encode Slice gate preparation result: {error}"))?
    );
    Ok(())
}

fn prepare_with(
    repository: &Path,
    prepare_path: &Path,
    output_path: &Path,
    verify: &VerifyReview<'_>,
    verify_carry: &VerifyCarry<'_>,
) -> Result<PrepareResult, String> {
    let prepare_bytes = bounded_file::read_regular(
        prepare_path,
        PREPARE_REQUEST_LIMIT,
        "Slice gate preparation request",
    )?;
    let request = parse_prepare_request(prepare_path, &prepare_bytes)?;
    let (gate_request, review, review_carry) =
        build_gate_request(repository, &request, verify, verify_carry)?;
    let gate_bytes = canonical_gate_request(&gate_request)?;

    run_final_revalidate_hook()?;
    let current_prepare = bounded_file::read_regular(
        prepare_path,
        PREPARE_REQUEST_LIMIT,
        "Slice gate preparation request",
    )?;
    if current_prepare != prepare_bytes {
        return Err("Slice gate preparation request changed before publication".to_owned());
    }
    let current_request = parse_prepare_request(prepare_path, &current_prepare)?;
    let (current_gate, current_review, current_carry) =
        build_gate_request(repository, &current_request, verify, verify_carry)?;
    if current_review != review
        || current_carry != review_carry
        || canonical_gate_request(&current_gate)? != gate_bytes
    {
        return Err("Slice gate preparation inputs changed before publication".to_owned());
    }

    let created = bounded_file::publish_new_or_exact(
        output_path,
        &gate_bytes,
        GATE_REQUEST_LIMIT,
        "prepared Slice gate request",
    )?;
    let gate = evaluate(repository, output_path)?;
    Ok(PrepareResult {
        schema: if review_carry.is_some() {
            RESULT_SCHEMA_V1_ALPHA1
        } else {
            RESULT_SCHEMA
        },
        ok: true,
        status: if created { "written" } else { "reused" },
        request_path: relative(repository, output_path),
        request_hash: digest(&gate_bytes),
        review_id: review.review_id,
        review_carry,
        gate,
    })
}

fn parse_prepare_request(path: &Path, bytes: &[u8]) -> Result<PrepareRequest, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
        format!(
            "invalid Slice gate preparation request {}: {error}",
            path.display()
        )
    })?;
    let review_carry_present = value
        .as_object()
        .is_some_and(|object| object.contains_key("review_carry"));
    let request: PrepareRequest = serde_json::from_value(value).map_err(|error| {
        format!(
            "invalid Slice gate preparation request {}: {error}",
            path.display()
        )
    })?;
    match request.schema.as_str() {
        model::REQUEST_SCHEMA if !review_carry_present => {},
        model::REQUEST_SCHEMA => {
            return Err(format!(
                "schema `{}` does not permit review_carry",
                model::REQUEST_SCHEMA
            ));
        },
        REQUEST_SCHEMA_V1_ALPHA1 if request.review_carry.is_some() => {},
        REQUEST_SCHEMA_V1_ALPHA1 => {
            return Err(format!(
                "schema `{REQUEST_SCHEMA_V1_ALPHA1}` requires review_carry"
            ));
        },
        other => {
            return Err(format!(
                "unsupported Slice gate preparation schema `{other}`; expected `{}` or `{REQUEST_SCHEMA_V1_ALPHA1}`",
                model::REQUEST_SCHEMA
            ));
        },
    }
    if request.validation_commands.len() > 32 {
        return Err("validation_commands exceeds the 32-entry limit".to_owned());
    }
    if request.review_runs.len() > 3 {
        return Err("review_runs exceeds the three-run limit".to_owned());
    }
    Ok(request)
}

fn build_gate_request(
    repository: &Path,
    request: &PrepareRequest,
    verify: &VerifyReview<'_>,
    verify_carry: &VerifyCarry<'_>,
) -> Result<
    (
        Request,
        VerifiedReview,
        Option<model::CanonicalApprovalReviewCarryResult>,
    ),
    String,
> {
    let bound = slice_contract::trusted_bound_slice(repository)?;
    slice_contract::trusted_check_bound_scope(repository)?;
    require_clean(repository)?;
    let candidate = trusted_line(repository, &["rev-parse", "--verify", "HEAD^{commit}"])?;

    let manifest_path = resolve_input_path(repository, &request.manifest_path);
    let manifest_bytes = bounded_file::read_regular(
        &manifest_path,
        MANIFEST_LIMIT,
        "published review-chain manifest",
    )?;
    let manifest_hash = digest(&manifest_bytes);
    let review = verify(repository, &manifest_path, &manifest_hash)?;
    if review.base_commit != bound.base {
        return Err(format!(
            "review chain base does not match bound Slice base {}",
            bound.base
        ));
    }
    let review_carry = request
        .review_carry
        .as_ref()
        .map(|carry| verify_carry(repository, &review, &candidate, carry))
        .transpose()?;
    if review_carry.is_none() && review.candidate_commit != candidate {
        return Err(format!(
            "review chain identity does not match bound Slice {}..{}",
            bound.base, candidate
        ));
    }
    require_review_contract(repository, &bound, &review)?;

    let changed = changed_paths(repository, &bound.base, &candidate)?;
    if changed.is_empty() {
        return Err("Slice gate preparation refuses a candidate with no changes".to_owned());
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
    let diff_hash = digest(&diff);
    let required = required_lenses(
        &review.review_lenses,
        &changed,
        bound.base_ref.starts_with("refs/heads/wave/"),
    )?;

    let validation_evidence = prepare_validation(
        &review,
        &request.validation_commands,
        &candidate,
        review_carry.is_some(),
    )?;
    let review_evidence = prepare_reviews(
        repository,
        &review,
        &request.review_runs,
        &candidate,
        &diff_hash,
    )?;
    let approval = request.approval.as_ref().map(|approval| Approval {
        kind: approval.kind.clone(),
        authority: approval.authority.clone(),
        scope: approval.scope.clone(),
        candidate_commit: (approval.kind == "exact_candidate").then(|| candidate.clone()),
        diff_hash: (approval.kind == "exact_candidate").then(|| diff_hash.clone()),
    });
    let gate = Request {
        schema: super::model::REQUEST_SCHEMA.to_owned(),
        candidate_commit: candidate.clone(),
        required_lenses: review.review_lenses.clone(),
        validation_evidence,
        review_evidence,
        known_unverified_environments: request.known_unverified_environments.clone(),
        risk: Risk {
            classification: request.risk.classification.clone(),
            rationale: request.risk.rationale.clone(),
        },
        approval,
    };

    validate_request_bounds(&gate)?;
    validation_results(repository, &gate, &candidate)?;
    review_results(repository, &gate, &candidate, &diff_hash, &required)?;
    approval_result(&gate.risk, gate.approval.as_ref(), &candidate, &diff_hash)?;
    Ok((gate, review, review_carry))
}

fn require_review_contract(
    repository: &Path,
    bound: &slice_contract::BoundSlice,
    review: &VerifiedReview,
) -> Result<(), String> {
    let reviewed_path = resolve_input_path(repository, &review.slice_contract_path);
    let reviewed = std::fs::canonicalize(&reviewed_path)
        .map_err(|error| format!("cannot resolve reviewed Slice contract: {error}"))?;
    let current = std::fs::canonicalize(&bound.contract_path)
        .map_err(|error| format!("cannot resolve bound Slice contract: {error}"))?;
    if reviewed != current || review.slice_contract_hash != bound.contract_id {
        return Err("review chain does not bind the current Slice contract".to_owned());
    }
    Ok(())
}

fn prepare_validation(
    review: &VerifiedReview,
    commands: &[ValidationCommand],
    candidate: &str,
    carried: bool,
) -> Result<Vec<ValidationEvidence>, String> {
    let mut by_name = BTreeMap::new();
    for command in commands {
        if by_name.insert(command.name.as_str(), command).is_some() {
            return Err(format!("duplicate validation command `{}`", command.name));
        }
    }
    let evidence_names = review
        .validation_evidence
        .iter()
        .map(|evidence| evidence.name.as_str())
        .collect::<BTreeSet<_>>();
    if by_name.keys().copied().collect::<BTreeSet<_>>() != evidence_names {
        return Err(
            "validation_commands must name every and only reviewed validation artifact".to_owned(),
        );
    }
    if carried && commands.iter().any(|command| !command.reused) {
        return Err(
            "canonical approval review carry requires every reviewed validation command to declare reused:true"
                .to_owned(),
        );
    }
    let mut prepared = review
        .validation_evidence
        .iter()
        .map(|evidence| {
            let command = by_name
                .get(evidence.name.as_str())
                .expect("validated command name exists");
            ValidationEvidence {
                name: evidence.name.clone(),
                argv: command.argv.clone(),
                result_path: evidence.path.clone(),
                result_hash: evidence.hash.clone(),
                candidate_commit: candidate.to_owned(),
                reused: command.reused,
            }
        })
        .collect::<Vec<_>>();
    prepared.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(prepared)
}

fn prepare_reviews(
    repository: &Path,
    review: &VerifiedReview,
    runs: &[model::ReviewRun],
    candidate: &str,
    diff_hash: &str,
) -> Result<Vec<ReviewEvidence>, String> {
    let mut prepared = Vec::new();
    for run in runs {
        let result_path = resolve_input_path(repository, &run.result_path);
        let result_bytes =
            bounded_file::read_regular(&result_path, REVIEW_RESULT_LIMIT, "review result")?;
        let result_hash = digest(&result_bytes);
        let route = match &run.source {
            ReviewSource::DeliveryReceipt {
                receipt_path,
                class,
            } => {
                let delivery = review_egress::verify_any_completed_delivery(
                    repository,
                    Path::new(receipt_path),
                    review,
                )?;
                match delivery {
                    review_egress::VerifiedDeliveryRoute::Managed {
                        provider,
                        model,
                        session_id,
                    } if matches!(class.as_str(), "model" | "model-high") => {
                        format!("{class}/{provider}/{model}/{session_id}")
                    },
                    review_egress::VerifiedDeliveryRoute::Delegated { host, session_id }
                        if matches!(class.as_str(), "delegated" | "delegated-high") =>
                    {
                        format!("{class}/{host}/{session_id}")
                    },
                    review_egress::VerifiedDeliveryRoute::Managed { .. } => {
                        return Err(
                            "managed delivery receipt class must be model or model-high".to_owned()
                        );
                    },
                    review_egress::VerifiedDeliveryRoute::Delegated { .. } => {
                        return Err(
                            "delegated delivery receipt class must be delegated or delegated-high"
                                .to_owned(),
                        );
                    },
                }
            },
            ReviewSource::DeclaredRoute { route } => route.clone(),
        };
        for verdict in &run.verdicts {
            let lens = Lens::parse(&verdict.lens)
                .ok_or_else(|| format!("unknown review lens `{}`", verdict.lens))?;
            let reviewer = review_coverage::reviewer_for_route(&route, lens)?;
            prepared.push(ReviewEvidence {
                lens: verdict.lens.clone(),
                reviewer,
                route: route.clone(),
                verdict: verdict.verdict.clone(),
                candidate_commit: candidate.to_owned(),
                diff_hash: diff_hash.to_owned(),
                result_path: run.result_path.clone(),
                result_hash: result_hash.clone(),
            });
        }
    }
    prepared.sort_by(|left, right| left.lens.cmp(&right.lens));
    Ok(prepared)
}

fn canonical_gate_request(request: &Request) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(request)
        .map_err(|error| format!("cannot encode prepared Slice gate request: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
type FinalRevalidateHook = Box<dyn FnOnce() -> Result<(), String>>;

#[cfg(test)]
thread_local! {
    static FINAL_REVALIDATE_HOOK: std::cell::RefCell<Option<FinalRevalidateHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_final_revalidate_hook(hook: impl FnOnce() -> Result<(), String> + 'static) {
    FINAL_REVALIDATE_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(test)]
fn run_final_revalidate_hook() -> Result<(), String> {
    let hook = FINAL_REVALIDATE_HOOK.with(|slot| slot.borrow_mut().take());
    hook.map_or(Ok(()), |hook| hook())
}

#[cfg(not(test))]
fn run_final_revalidate_hook() -> Result<(), String> {
    Ok(())
}
