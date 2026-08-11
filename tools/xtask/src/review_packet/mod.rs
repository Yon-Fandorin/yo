mod canonical;
mod capture;
mod model;
mod render;
mod revalidate;
pub(crate) mod storage;
mod trusted_git;
mod verifier;

#[cfg(test)]
pub(crate) mod tests;

use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
};

use self::{
    canonical::{build_manifest, build_plan, delivery_profile_bytes},
    capture::{
        Inputs, capture_authorities, capture_context, capture_diff, capture_validation, captured,
    },
    model::{
        Artifact, ArtifactWithTokens, DELIVERY_PROFILE, PREFLIGHT_RESULT_SCHEMA, PreflightPacket,
        PreflightResultRecord, REQUEST_SCHEMA, RESULT_SCHEMA, Request, ResultRecord, ReviewPlan,
        SECTION_TOKEN_ACCOUNTING, TOKENIZER_PROFILE,
    },
    render::{count_tokens, render_packet, render_packet_with_measurements, require_budget},
    revalidate::final_revalidate,
    trusted_git::{
        expected_slice_ref, trusted_ensure_clean, trusted_git_succeeds, trusted_git_text,
        trusted_repository_root, trusted_resolve_commit,
    },
};
use crate::{
    bounded_file,
    review_protocol::{
        Captured, digest, domain_digest, relative, require_commit, resolve_input_path,
        sorted_unique,
    },
    slice_contract,
};

const REVIEW_ID_DOMAIN: &[u8] = b"yo.slice-review/v1";
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PACKET_BYTES: usize = 32 * 1024 * 1024;
const PREAMBLE: &str = "# yo Slice Review Packet\n\nThis packet is the complete caller-controlled model-visible review payload. Review only the immutable candidate described here against the included authority, contract, evidence, and questions.\n";
const SECTION_PREFIX: &str = "\n<<<YO-REVIEW-SECTION ";
const METADATA_SUFFIX: &str = ">>>\n";
const SECTION_SUFFIX: &str = "\n<<<YO-REVIEW-SECTION-END>>>\n";
const PAYLOAD_SUFFIX: &str = "\n<<<YO-REVIEW-PAYLOAD-END>>>\n";

pub(crate) use verifier::{VerifiedEvidence, VerifiedReview, verify_published};

pub(super) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let PreparedReview {
        repository,
        request,
        inputs,
        plan,
        review_id,
    } = prepare_review(repository, request_path)?;
    let packet = render_packet(&review_id, &plan, &inputs)?;
    let managed_payload_tokens = require_packet_budget(&packet, inputs.max_tokens)?;
    let packet_hash = digest(&packet);
    let manifest = build_manifest(
        review_id.clone(),
        plan,
        &inputs,
        packet_hash.clone(),
        managed_payload_tokens,
    );
    let mut manifest_bytes =
        serde_json::to_vec_pretty(&manifest).expect("closed review manifest serializes");
    manifest_bytes.push(b'\n');
    let manifest_hash = digest(&manifest_bytes);
    let suffix = review_id
        .strip_prefix("sha256:")
        .expect("generated ReviewId has a sha256 prefix");
    let output_directory = repository
        .join(".local-exclude/methexis/slice-reviews")
        .join(suffix);
    let status = storage::publish(&output_directory, &packet, &manifest_bytes, || {
        final_revalidate(&repository, &request, &inputs)
    })?;

    write_result(
        &ResultRecord {
            schema: RESULT_SCHEMA,
            ok: true,
            operation: "build_slice_review_packet",
            status,
            review_id,
            trusted_commit: inputs.context.result.trusted_commit.clone(),
            candidate_commit: inputs.candidate_commit.clone(),
            packet: ArtifactWithTokens {
                path: relative(&repository, &output_directory.join("packet.md")),
                hash: packet_hash,
                managed_payload_tokens,
            },
            manifest: Artifact {
                path: relative(&repository, &output_directory.join("manifest.json")),
                hash: manifest_hash,
            },
            max_managed_payload_tokens: inputs.max_tokens,
        },
        "review packet result",
    )
}

pub(super) fn preflight(
    repository: &Path,
    request_path: &Path,
    output: &mut impl io::Write,
) -> Result<(), String> {
    let PreparedReview {
        repository,
        request,
        inputs,
        plan,
        review_id,
    } = prepare_review(repository, request_path)?;
    let rendered = render_packet_with_measurements(&review_id, &plan, &inputs)?;
    let managed_payload_tokens = require_packet_budget(&rendered.bytes, inputs.max_tokens)?;
    run_preflight_test_hook()?;
    final_revalidate(&repository, &request, &inputs)?;

    write_result_to(
        output,
        &PreflightResultRecord {
            schema: PREFLIGHT_RESULT_SCHEMA,
            ok: true,
            operation: "preflight_slice_review_packet",
            status: "ready",
            artifacts_published: false,
            review_id,
            trusted_commit: inputs.context.result.trusted_commit.clone(),
            candidate_commit: inputs.candidate_commit.clone(),
            packet: PreflightPacket {
                bytes: rendered.bytes.len(),
                managed_payload_tokens,
                max_managed_payload_tokens: inputs.max_tokens,
            },
            section_token_accounting: SECTION_TOKEN_ACCOUNTING,
            sections: rendered.sections,
        },
        "review packet preflight result",
    )
}

struct PreparedReview {
    repository: PathBuf,
    request: Request,
    inputs: Inputs,
    plan: ReviewPlan,
    review_id: String,
}

fn prepare_review(repository: &Path, request_path: &Path) -> Result<PreparedReview, String> {
    let repository = trusted_repository_root(repository)?;
    let request_bytes = bounded_file::read_regular(
        request_path,
        MAX_REQUEST_BYTES,
        "Slice review packet request",
    )?;
    let request: Request = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("invalid Slice review packet request: {error}"))?;
    validate_request(&request)?;
    trusted_ensure_clean(
        &repository,
        "candidate worktree",
        "building a review packet",
    )?;

    let candidate_commit = trusted_resolve_commit(&repository, "HEAD")?;
    let contract_path = resolve_input_path(&repository, &request.slice_contract_path);
    let contract_bytes =
        bounded_file::read_regular(&contract_path, MAX_REQUEST_BYTES, "Slice review contract")?;
    let bound = slice_contract::trusted_bound_slice(&repository)?;
    let canonical_contract = std::fs::canonicalize(&contract_path)
        .map_err(|error| format!("cannot resolve Slice contract: {error}"))?;
    if canonical_contract != bound.contract_path || digest(&contract_bytes) != bound.contract_id {
        return Err("request does not identify the exact bound Slice contract".to_owned());
    }
    let branch = trusted_git_text(&repository, &["symbolic-ref", "--quiet", "HEAD"])?;
    let expected_branch = expected_slice_ref(&bound.base_ref, &bound.slice)?;
    if branch.trim() != expected_branch {
        return Err(format!(
            "trusted Git branch does not match bound Slice; expected {expected_branch}"
        ));
    }
    require_commit(&bound.base, "Slice base")?;
    let base_commit = trusted_resolve_commit(&repository, &bound.base)?;
    if !trusted_git_succeeds(
        &repository,
        &[
            "merge-base",
            "--is-ancestor",
            &base_commit,
            &candidate_commit,
        ],
    )? {
        return Err("Slice base is not an ancestor of the candidate commit".to_owned());
    }

    let context_request_path = resolve_input_path(&repository, &request.context_request_path);
    let context = capture_context(&repository, &context_request_path)?;
    let required_knowledge_ids =
        sorted_unique(&request.required_knowledge_ids, "required KnowledgeId")?;
    let included = context.included_ids.iter().collect::<BTreeSet<_>>();
    if required_knowledge_ids
        .iter()
        .any(|knowledge_id| !included.contains(knowledge_id))
    {
        return Err("ContextBuild does not include every required KnowledgeId".to_owned());
    }
    let diff_bytes = capture_diff(&repository, &base_commit, &candidate_commit)?;
    let diff = captured("git-diff.patch".to_owned(), diff_bytes)?;
    let authorities = capture_authorities(
        &repository,
        &candidate_commit,
        &request.repository_authority_paths,
    )?;
    let validation = capture_validation(&repository, &request.validation_evidence)?;
    let slice_contract = Captured {
        path: contract_path.to_string_lossy().into_owned(),
        hash: digest(&contract_bytes),
        bytes: contract_bytes,
    };
    let inputs = Inputs {
        base_commit,
        candidate_commit,
        diff,
        context,
        authorities,
        slice_contract,
        validation,
        lenses: sorted_unique(&request.review_lenses, "review lens")?,
        questions: request.review_questions.clone(),
        required_knowledge_ids,
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: request.max_managed_payload_tokens,
    };

    let plan = build_plan(&inputs);
    let plan_bytes = serde_json::to_vec(&plan).expect("closed review plan serializes");
    let review_id = domain_digest(REVIEW_ID_DOMAIN, &plan_bytes);
    Ok(PreparedReview {
        repository,
        request,
        inputs,
        plan,
        review_id,
    })
}

fn require_packet_budget(packet: &[u8], max_tokens: usize) -> Result<usize, String> {
    if packet.len() > MAX_PACKET_BYTES {
        return Err(format!(
            "canonical review packet exceeds the {MAX_PACKET_BYTES}-byte safety limit"
        ));
    }
    let managed_payload_tokens = count_tokens(packet)?;
    require_budget(managed_payload_tokens, max_tokens)?;
    Ok(managed_payload_tokens)
}

fn write_result(result: &impl serde::Serialize, label: &str) -> Result<(), String> {
    write_result_to(&mut io::stdout().lock(), result, label)
}

fn write_result_to(
    output: &mut impl io::Write,
    result: &impl serde::Serialize,
    label: &str,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(result).expect("closed review result serializes");
    bytes.push(b'\n');
    io::Write::write_all(output, &bytes).map_err(|error| format!("cannot write {label}: {error}"))
}

#[cfg(test)]
type PreflightTestHook = Box<dyn FnOnce() -> Result<(), String>>;

#[cfg(test)]
thread_local! {
    static PREFLIGHT_TEST_HOOK: std::cell::RefCell<Option<PreflightTestHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_preflight_test_hook(hook: impl FnOnce() -> Result<(), String> + 'static) {
    PREFLIGHT_TEST_HOOK.with(|slot| {
        assert!(slot.replace(Some(Box::new(hook))).is_none());
    });
}

#[cfg(test)]
fn run_preflight_test_hook() -> Result<(), String> {
    let hook = PREFLIGHT_TEST_HOOK.with(|slot| slot.borrow_mut().take());
    hook.map_or(Ok(()), |hook| hook())
}

#[cfg(not(test))]
fn run_preflight_test_hook() -> Result<(), String> {
    Ok(())
}

fn validate_request(request: &Request) -> Result<(), String> {
    if request.schema != REQUEST_SCHEMA {
        return Err(format!("expected request schema `{REQUEST_SCHEMA}`"));
    }
    if request.delivery_profile != DELIVERY_PROFILE {
        return Err(format!("expected delivery profile `{DELIVERY_PROFILE}`"));
    }
    if request.tokenizer_profile != TOKENIZER_PROFILE {
        return Err(format!("expected tokenizer profile `{TOKENIZER_PROFILE}`"));
    }
    if request.max_managed_payload_tokens == 0 {
        return Err("managed payload token budget must be positive".to_owned());
    }
    if request.repository_authority_paths.is_empty()
        || request.validation_evidence.is_empty()
        || request.required_knowledge_ids.is_empty()
        || request.review_lenses.is_empty()
        || request.review_questions.is_empty()
    {
        return Err(
            "authority paths, validation evidence, review lenses, and questions must be non-empty"
                .to_owned(),
        );
    }
    if request
        .review_questions
        .iter()
        .any(|question| question.trim().is_empty())
    {
        return Err("review questions must not be blank".to_owned());
    }
    Ok(())
}
