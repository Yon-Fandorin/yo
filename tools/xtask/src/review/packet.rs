mod bootstrap;
mod canonical;
mod capture;
pub(crate) mod external_operation;
mod model;
mod readiness;
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
    bootstrap::require_prospective_activation_boundary,
    canonical::{build_manifest, build_plan, delivery_profile_bytes_for_id},
    capture::{
        Inputs, capture_authorities, capture_context_request, capture_context_with_request,
        capture_diff, capture_prospective_context_with_request, capture_validation, captured,
    },
    model::{
        Artifact, ArtifactWithTokens, DELIVERY_PROFILE_V1, DELIVERY_PROFILE_V1_ALPHA1,
        DELIVERY_PROFILE_V1_ALPHA2, DELIVERY_PROFILE_V1_ALPHA3, PREFLIGHT_RESULT_SCHEMA_V1,
        PREFLIGHT_RESULT_SCHEMA_V1_ALPHA1, PREFLIGHT_RESULT_SCHEMA_V1_ALPHA2,
        PREFLIGHT_RESULT_SCHEMA_V1_ALPHA3, PreflightPacket, PreflightResultRecord, REQUEST_SCHEMA,
        REQUEST_SCHEMA_V1_ALPHA3, RESULT_SCHEMA, RESULT_SCHEMA_V1_ALPHA3, Request, ResultRecord,
        ReviewPlan, SECTION_TOKEN_ACCOUNTING, TOKENIZER_PROFILE,
    },
    render::{
        count_tokens, render_packet_with_measurements, render_packet_with_metadata, require_budget,
    },
    revalidate::final_revalidate,
    trusted_git::{
        expected_slice_ref, trusted_ensure_clean, trusted_git_succeeds, trusted_git_text,
        trusted_repository_root, trusted_resolve_commit,
    },
};
use crate::{
    bounded_file,
    review_protocol::{
        Captured, NamedCaptured, digest, domain_digest, relative, require_commit,
        resolve_input_path, sorted_unique,
    },
    slice_contract,
};

const REVIEW_ID_DOMAIN: &[u8] = b"yo.slice-review/v1";
const REVIEW_ID_DOMAIN_V1_ALPHA3: &[u8] = b"yo.slice-review/v1alpha3";
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PACKET_BYTES: usize = 32 * 1024 * 1024;
const PREAMBLE: &str = "# yo Slice Review Packet\n\nThis packet is the complete caller-controlled model-visible review payload. Review only the immutable candidate described here against the included authority, contract, evidence, and questions.\n";
const PREAMBLE_V1_ALPHA1: &str = "# yo Slice Review Packet\n\nThis packet is the complete caller-controlled model-visible review payload. Do not return a verdict unless the full review plan, exact base-to-candidate diff, and final YO-REVIEW-PAYLOAD-END marker are present. Review only that immutable candidate against the included authority, contract, evidence, and questions.\n";
const PREAMBLE_V1_ALPHA2: &str = "# yo Slice Review Packet\n\nThis packet is the complete caller-controlled model-visible review payload. Do not return a verdict unless the full review plan, exact base-to-candidate diff, and final YO-REVIEW-PAYLOAD-END marker are present. A section whose metadata names yo.slice-review-sentinel-escape/v1 is reversible model-visible text: every literal backslash is doubled, and the first less-than byte of any literal wrapper-sentinel prefix is written as ASCII backslash-x3c. Interpret that decoded content when reviewing. Review only the immutable candidate against the included authority, contract, evidence, and questions.\n";
const PREAMBLE_V1_ALPHA3: &str = "# yo Prospective Activation Review Packet\n\nThis packet is review-only and its ContextBuild authority is prospective, not active or trusted. Do not return a verdict unless the exact activation request, proposed Checkpoint, canonical active-record transition, complete review plan, base-to-candidate diff, and final YO-REVIEW-PAYLOAD-END marker are present. A section whose metadata names yo.slice-review-sentinel-escape/v1 is reversible model-visible text: every literal backslash is doubled, and the first less-than byte of any literal wrapper-sentinel prefix is written as ASCII backslash-x3c. Interpret that decoded content when reviewing. This packet grants no approval, activation, or general ContextBuild eligibility.\n";
const SECTION_PREFIX: &str = "\n<<<YO-REVIEW-SECTION ";
const METADATA_SUFFIX: &str = ">>>\n";
const SECTION_SUFFIX: &str = "\n<<<YO-REVIEW-SECTION-END>>>\n";
const PAYLOAD_SUFFIX: &str = "\n<<<YO-REVIEW-PAYLOAD-END>>>\n";

pub(crate) use verifier::{VerifiedEvidence, VerifiedReview, verify_published};

#[derive(Clone, Debug)]
pub(crate) struct PublishedReview {
    pub(crate) status: &'static str,
    pub(crate) schema: &'static str,
    pub(crate) authority: Option<&'static str>,
    pub(crate) review_id: String,
    pub(crate) trusted_commit: String,
    pub(crate) candidate_commit: String,
    pub(crate) packet_path: String,
    pub(crate) packet_hash: String,
    pub(crate) packet_bytes: usize,
    pub(crate) managed_payload_tokens: usize,
    pub(crate) manifest_path: String,
    pub(crate) manifest_hash: String,
    pub(crate) max_managed_payload_tokens: usize,
}

pub(crate) fn is_original_manifest_schema(schema: &str) -> bool {
    matches!(
        schema,
        model::MANIFEST_SCHEMA_V1
            | model::MANIFEST_SCHEMA_V1_ALPHA1
            | model::MANIFEST_SCHEMA_V1_ALPHA2
            | model::MANIFEST_SCHEMA_V1_ALPHA3
    )
}

pub(crate) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let published = publish(repository, request_path)?;
    write_result(
        &ResultRecord {
            schema: published.schema,
            ok: true,
            operation: "build_slice_review_packet",
            status: published.status,
            authority: published.authority,
            review_id: published.review_id,
            trusted_commit: published.trusted_commit,
            candidate_commit: published.candidate_commit,
            packet: ArtifactWithTokens {
                path: published.packet_path,
                hash: published.packet_hash,
                managed_payload_tokens: published.managed_payload_tokens,
            },
            manifest: Artifact {
                path: published.manifest_path,
                hash: published.manifest_hash,
            },
            max_managed_payload_tokens: published.max_managed_payload_tokens,
        },
        "review packet result",
    )
}

pub(crate) fn publish(repository: &Path, request_path: &Path) -> Result<PublishedReview, String> {
    let PreparedReview {
        repository,
        request,
        inputs,
        plan,
        review_id,
    } = prepare_review(repository, request_path)?;
    let rendered = render_packet_with_metadata(&review_id, &plan, &inputs)?;
    let managed_payload_tokens = require_packet_budget(&rendered.bytes, inputs.max_tokens)?;
    let packet_bytes = rendered.bytes.len();
    let packet_hash = digest(&rendered.bytes);
    let delivery_profile_id = plan.delivery_profile.id.clone();
    let manifest = build_manifest(
        review_id.clone(),
        plan,
        &inputs,
        packet_hash.clone(),
        managed_payload_tokens,
        rendered.input_prefix,
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
    let status = storage::publish(&output_directory, &rendered.bytes, &manifest_bytes, || {
        final_revalidate(&repository, &request, &inputs)
    })?;

    Ok(PublishedReview {
        status,
        schema: result_schema(&delivery_profile_id)?,
        authority: authority_label(&delivery_profile_id),
        review_id,
        trusted_commit: inputs.context.result.trusted_commit.clone(),
        candidate_commit: inputs.candidate_commit.clone(),
        packet_path: relative(&repository, &output_directory.join("packet.md")),
        packet_hash,
        packet_bytes,
        managed_payload_tokens,
        manifest_path: relative(&repository, &output_directory.join("manifest.json")),
        manifest_hash,
        max_managed_payload_tokens: inputs.max_tokens,
    })
}

pub(crate) fn preflight(
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
            schema: preflight_result_schema(&plan.delivery_profile.id)?,
            ok: true,
            operation: "preflight_slice_review_packet",
            status: "ready",
            artifacts_published: false,
            authority: authority_label(&plan.delivery_profile.id),
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
            input_prefix: rendered.input_prefix,
        },
        "review packet preflight result",
    )
}

pub(crate) fn check_readiness(
    repository: &Path,
    request_path: &Path,
    output: &mut impl io::Write,
) -> Result<(), String> {
    readiness::run(repository, request_path, output)
}

struct PreparedReview {
    repository: PathBuf,
    request: Request,
    inputs: Inputs,
    plan: ReviewPlan,
    review_id: String,
}

struct PreparedReadiness {
    repository: PathBuf,
    request_capture: Captured,
    request: Request,
    slice: String,
    base_commit: String,
    trusted_commit: String,
    candidate_commit: String,
    slice_contract_request_path: PathBuf,
    slice_contract: Captured,
    context_request: Captured,
    activation_request: Option<Captured>,
    authorities: Vec<Captured>,
    validation: Vec<NamedCaptured>,
    required_knowledge_ids: Vec<String>,
    lenses: Vec<String>,
}

fn prepare_review(repository: &Path, request_path: &Path) -> Result<PreparedReview, String> {
    let readiness = prepare_readiness(repository, request_path, "building a review packet")?;
    let context_request_path = Path::new(&readiness.context_request.path);
    let (context, prospective) = match readiness.activation_request.clone() {
        Some(activation_request) => {
            let activation_request_path = PathBuf::from(&activation_request.path);
            let (context, prospective) = capture_prospective_context_with_request(
                &readiness.repository,
                &readiness.candidate_commit,
                &activation_request_path,
                activation_request,
                context_request_path,
                readiness.context_request.clone(),
            )?;
            (context, Some(prospective))
        },
        None => (
            capture_context_with_request(
                &readiness.repository,
                context_request_path,
                readiness.context_request.clone(),
            )?,
            None,
        ),
    };
    let included = context.included_ids.iter().collect::<BTreeSet<_>>();
    if readiness
        .required_knowledge_ids
        .iter()
        .any(|knowledge_id| !included.contains(knowledge_id))
    {
        return Err("ContextBuild does not include every required KnowledgeId".to_owned());
    }
    let diff_bytes = capture_diff(
        &readiness.repository,
        &readiness.base_commit,
        &readiness.candidate_commit,
    )?;
    let diff = captured("git-diff.patch".to_owned(), diff_bytes)?;
    let inputs = Inputs {
        base_commit: readiness.base_commit,
        candidate_commit: readiness.candidate_commit,
        diff,
        context,
        prospective,
        authorities: readiness.authorities,
        slice_contract: readiness.slice_contract,
        validation: readiness.validation,
        lenses: readiness.lenses,
        questions: readiness.request.review_questions.clone(),
        required_knowledge_ids: readiness.required_knowledge_ids,
        delivery_profile_bytes: delivery_profile_bytes_for_id(&readiness.request.delivery_profile)?,
        max_tokens: readiness.request.max_managed_payload_tokens,
    };

    let plan = build_plan(&inputs);
    let plan_bytes = serde_json::to_vec(&plan).expect("closed review plan serializes");
    let review_id = domain_digest(review_id_domain(&plan.delivery_profile.id), &plan_bytes);
    Ok(PreparedReview {
        repository: readiness.repository,
        request: readiness.request,
        inputs,
        plan,
        review_id,
    })
}

fn prepare_readiness(
    repository: &Path,
    request_path: &Path,
    operation: &str,
) -> Result<PreparedReadiness, String> {
    let request_path = if request_path.is_absolute() {
        request_path.to_owned()
    } else {
        repository.join(request_path)
    };
    let repository = trusted_repository_root(repository)?;
    let request_bytes = bounded_file::read_regular(
        &request_path,
        MAX_REQUEST_BYTES,
        "Slice review packet request",
    )?;
    let request_capture = captured(
        request_path.to_string_lossy().into_owned(),
        request_bytes.clone(),
    )?;
    let request: Request = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("invalid Slice review packet request: {error}"))?;
    validate_request(&request)?;
    trusted_ensure_clean(&repository, "candidate worktree", operation)?;

    let candidate_commit = trusted_resolve_commit(&repository, "HEAD")?;
    let trusted_commit = trusted_resolve_commit(&repository, "refs/heads/develop")?;
    let contract_path = resolve_input_path(&repository, &request.slice_contract_path);
    let contract_bytes =
        bounded_file::read_regular(&contract_path, MAX_REQUEST_BYTES, "Slice review contract")?;
    let bound = slice_contract::trusted_bound_slice(&repository)?;
    let canonical_contract = std::fs::canonicalize(&contract_path)
        .map_err(|error| format!("cannot resolve Slice contract: {error}"))?;
    if canonical_contract != bound.contract_path || digest(&contract_bytes) != bound.contract_id {
        return Err("request does not identify the exact bound Slice contract".to_owned());
    }
    require_exact_slice_branch(&repository, &bound)?;
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
    slice_contract::trusted_check_bound_scope(&repository)?;

    let context_request_path = resolve_input_path(&repository, &request.context_request_path);
    let context_request = capture_context_request(&repository, &context_request_path)?;
    let activation_request = request
        .activation_request_path
        .as_deref()
        .map(|path| {
            let path = resolve_input_path(&repository, path);
            capture_context_request(&repository, &path)
        })
        .transpose()?;
    if let Some(activation_request) = &activation_request {
        require_prospective_activation_boundary(
            &repository,
            &trusted_commit,
            &candidate_commit,
            activation_request,
        )?;
    }
    let required_knowledge_ids =
        sorted_unique(&request.required_knowledge_ids, "required KnowledgeId")?;
    let authorities = capture_authorities(
        &repository,
        &candidate_commit,
        &request.repository_authority_paths,
    )?;
    let validation =
        capture_validation(&repository, &candidate_commit, &request.validation_evidence)?;
    let slice_contract = Captured {
        path: canonical_contract.to_string_lossy().into_owned(),
        hash: digest(&contract_bytes),
        bytes: contract_bytes,
    };
    let lenses = sorted_unique(&request.review_lenses, "review lens")?;
    Ok(PreparedReadiness {
        repository,
        request_capture,
        request,
        slice: bound.slice,
        base_commit,
        trusted_commit,
        candidate_commit,
        slice_contract_request_path: contract_path,
        slice_contract,
        context_request,
        activation_request,
        authorities,
        validation,
        lenses,
        required_knowledge_ids,
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

fn require_exact_slice_branch(
    repository: &Path,
    bound: &slice_contract::BoundSlice,
) -> Result<(), String> {
    let branch = trusted_git_text(repository, &["symbolic-ref", "--quiet", "HEAD"])?;
    let expected_branch = expected_slice_ref(&bound.base_ref, &bound.slice)?;
    if branch.trim() == expected_branch {
        Ok(())
    } else {
        Err(format!(
            "trusted Git branch does not match bound Slice; expected {expected_branch}"
        ))
    }
}

fn validate_request(request: &Request) -> Result<(), String> {
    match request.schema.as_str() {
        REQUEST_SCHEMA => {
            if request.activation_request_path.is_some()
                || !matches!(
                    request.delivery_profile.as_str(),
                    DELIVERY_PROFILE_V1 | DELIVERY_PROFILE_V1_ALPHA1 | DELIVERY_PROFILE_V1_ALPHA2
                )
            {
                return Err("ordinary review requests must omit activation_request_path and use an ordinary delivery profile".to_owned());
            }
        },
        REQUEST_SCHEMA_V1_ALPHA3 => {
            if request.activation_request_path.is_none()
                || request.delivery_profile != DELIVERY_PROFILE_V1_ALPHA3
            {
                return Err(format!(
                    "prospective activation review requests must name activation_request_path and use delivery profile `{DELIVERY_PROFILE_V1_ALPHA3}`"
                ));
            }
        },
        _ => {
            return Err(format!(
                "expected request schema `{REQUEST_SCHEMA}` or `{REQUEST_SCHEMA_V1_ALPHA3}`"
            ));
        },
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
    let mut questions = BTreeSet::new();
    for question in &request.review_questions {
        if question.trim().is_empty() {
            return Err("review questions must not be blank".to_owned());
        }
        if !questions.insert(question) {
            return Err("review questions must be unique".to_owned());
        }
    }
    Ok(())
}

fn preflight_result_schema(profile: &str) -> Result<&'static str, String> {
    match profile {
        DELIVERY_PROFILE_V1 => Ok(PREFLIGHT_RESULT_SCHEMA_V1),
        DELIVERY_PROFILE_V1_ALPHA1 => Ok(PREFLIGHT_RESULT_SCHEMA_V1_ALPHA1),
        DELIVERY_PROFILE_V1_ALPHA2 => Ok(PREFLIGHT_RESULT_SCHEMA_V1_ALPHA2),
        DELIVERY_PROFILE_V1_ALPHA3 => Ok(PREFLIGHT_RESULT_SCHEMA_V1_ALPHA3),
        _ => Err(format!(
            "unsupported original review delivery profile `{profile}`"
        )),
    }
}

fn result_schema(profile: &str) -> Result<&'static str, String> {
    if profile == DELIVERY_PROFILE_V1_ALPHA3 {
        Ok(RESULT_SCHEMA_V1_ALPHA3)
    } else if matches!(
        profile,
        DELIVERY_PROFILE_V1 | DELIVERY_PROFILE_V1_ALPHA1 | DELIVERY_PROFILE_V1_ALPHA2
    ) {
        Ok(RESULT_SCHEMA)
    } else {
        Err(format!(
            "unsupported original review delivery profile `{profile}`"
        ))
    }
}

fn authority_label(profile: &str) -> Option<&'static str> {
    (profile == DELIVERY_PROFILE_V1_ALPHA3).then_some("prospective")
}

fn review_id_domain(profile: &str) -> &'static [u8] {
    if profile == DELIVERY_PROFILE_V1_ALPHA3 {
        REVIEW_ID_DOMAIN_V1_ALPHA3
    } else {
        REVIEW_ID_DOMAIN
    }
}
