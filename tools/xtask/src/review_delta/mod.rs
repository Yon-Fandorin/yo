mod capture;
mod chain;
mod evidence;
mod git_state;
mod model;
mod render;
mod request;
mod v1;
mod v1alpha1;

pub(crate) use chain::verify_chain_head;

#[cfg(test)]
mod tests;

use std::{collections::BTreeSet, io, path::Path};

use self::{
    capture::{
        capture_published, captured, require_current_file, require_current_packet,
        require_named_captures,
    },
    evidence::{
        TransitionContext, capture_prior_findings, capture_validation, require_exact_finding_set,
        sorted_findings, validate_transition,
    },
    git_state::{
        capture_delta, require_expected_branch, trusted_ensure_clean, trusted_repository_root,
        trusted_resolve_commit,
    },
    model::{Artifact, ArtifactWithTokens, FindingDisposition, Request, ResultRecord},
    render::{
        build_manifest_for, build_plan_for, count_tokens, delivery_profile_bytes_for,
        render_packet, require_budget,
    },
    request::{validate_prior, validate_request},
};
use crate::{
    bounded_file,
    review_packet::{VerifiedReview, storage},
    review_protocol::{
        Captured, NamedCaptured, digest, domain_digest, relative, resolve_input_path,
    },
    slice_contract,
};

const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PACKET_BYTES: usize = 32 * 1024 * 1024;
const MAX_AGGREGATE_EVIDENCE_BYTES: usize = 32 * 1024 * 1024;
const REQUEST_SCHEMA_V1_ALPHA2: &str = "yo.slice-review-delta-request/v1alpha2";
const PREAMBLE: &str = "# yo Slice Finding-Resolution Review Delta\n\nThis is the complete caller-controlled continuation payload for the same reviewer, lens, and scope. Identify the prior ReviewId and candidate from your existing session, verify that the supplied prior-findings artifact is the exact finding set you issued, then inspect only the replacement delta and supplied dispositions/evidence. Fail closed if that prior context or exact finding set is unavailable.\n";
const SECTION_PREFIX: &str = "\n<<<YO-REVIEW-DELTA-SECTION ";
const METADATA_SUFFIX: &str = ">>>\n";
const SECTION_SUFFIX: &str = "\n<<<YO-REVIEW-DELTA-SECTION-END>>>\n";
const PAYLOAD_SUFFIX: &str = "\n<<<YO-REVIEW-DELTA-PAYLOAD-END>>>\n";

#[derive(Clone, Copy)]
enum AffectedPathPolicy {
    LegacyStringIdentity,
    CanonicalIdentity,
}

#[derive(Clone, Copy)]
struct WireContract {
    plan_schema: &'static str,
    manifest_schema: &'static str,
    delivery_profile: &'static str,
    review_id_domain: &'static [u8],
    affected_path_policy: AffectedPathPolicy,
}

struct Inputs {
    request: Captured,
    prior_manifest: Captured,
    prior_packet: Captured,
    prior_findings: Captured,
    prior: VerifiedReview,
    replacement_candidate: String,
    delta: Captured,
    slice_contract: Captured,
    findings: Vec<FindingDisposition>,
    reused_validation: Vec<NamedCaptured>,
    affected_validation: Vec<NamedCaptured>,
    delivery_profile_bytes: Vec<u8>,
    max_tokens: usize,
}

pub(super) fn run(repository: &Path, request_path: &Path) -> Result<(), String> {
    let repository = trusted_repository_root(repository)?;
    let request_bytes = bounded_file::read_regular(
        request_path,
        MAX_REQUEST_BYTES,
        "Slice review delta request",
    )?;
    let mut request: Request = serde_json::from_slice(&request_bytes)
        .map_err(|error| format!("invalid Slice review delta request: {error}"))?;
    let verify_affected_identity = normalize_request_schema(&mut request);
    validate_request(&request)?;
    let contract = v1alpha1::contract();
    trusted_ensure_clean(&repository, "building a review delta")?;

    let bound = slice_contract::trusted_bound_slice(&repository)?;
    require_expected_branch(&repository, &bound.base_ref, &bound.slice)?;
    let replacement_candidate = trusted_resolve_commit(&repository, "HEAD")?;
    let manifest_path = resolve_input_path(&repository, &request.prior_manifest_path);
    let prior = verify_chain_head(
        &repository,
        &manifest_path,
        &request.prior_manifest_hash,
        &mut BTreeSet::new(),
        0,
    )?;
    validate_prior(&prior, &bound, &replacement_candidate)?;
    slice_contract::trusted_check_bound_scope(&repository)?;
    let prior_manifest = capture_published(
        &repository,
        &resolve_input_path(&repository, &prior.manifest_path),
        "prior review manifest",
        MAX_INPUT_BYTES,
    )?;
    let prior_packet = capture_published(
        &repository,
        &resolve_input_path(&repository, &prior.packet_path),
        "prior review packet",
        MAX_PACKET_BYTES,
    )?;
    if prior_manifest.hash != prior.manifest_hash || prior_packet.hash != prior.packet_hash {
        return Err("verified prior review artifact identities differ".to_owned());
    }
    let prior_findings = capture_prior_findings(&repository, &request, &prior)?;

    let contract_bytes = bounded_file::read_regular(
        &bound.contract_path,
        MAX_REQUEST_BYTES,
        "bound Slice contract",
    )?;
    let slice_contract = captured(
        bound.contract_path.to_string_lossy().into_owned(),
        contract_bytes,
    )?;
    if slice_contract.hash != bound.contract_id
        || prior.slice_contract_path != slice_contract.path
        || prior.slice_contract_hash != slice_contract.hash
    {
        return Err("prior review and current bound Slice contract differ".to_owned());
    }

    let delta = captured(
        "git-delta.patch".to_owned(),
        capture_delta(&repository, &prior.candidate_commit, &replacement_candidate)?,
    )?;
    if delta.bytes.is_empty() {
        return Err("replacement candidate has no delta from the prior candidate".to_owned());
    }
    let findings = sorted_findings(&request.finding_dispositions)?;
    require_exact_finding_set(&prior_findings, &findings)?;
    let (reused_validation, affected_validation) = capture_validation(
        &repository,
        &prior,
        &replacement_candidate,
        verify_affected_identity,
        &request.reused_validation_evidence,
        &request.affected_validation_evidence,
    )?;
    validate_transition(
        TransitionContext::new(&repository, contract.affected_path_policy),
        &prior,
        &replacement_candidate,
        &delta,
        &findings,
        &reused_validation,
        &affected_validation,
    )?;
    let inputs = Inputs {
        request: captured(request_path.to_string_lossy().into_owned(), request_bytes)?,
        prior_manifest,
        prior_packet,
        prior_findings,
        prior,
        replacement_candidate,
        delta,
        slice_contract,
        findings,
        reused_validation,
        affected_validation,
        delivery_profile_bytes: delivery_profile_bytes_for(contract),
        max_tokens: request.max_managed_payload_tokens,
    };

    let plan = build_plan_for(&inputs, contract);
    let plan_bytes = serde_json::to_vec(&plan).expect("closed review delta plan serializes");
    let review_delta_id = domain_digest(contract.review_id_domain, &plan_bytes);
    let packet = render_packet(&review_delta_id, &plan, &inputs)?;
    if packet.len() > MAX_PACKET_BYTES {
        return Err(format!(
            "canonical review delta packet exceeds the {MAX_PACKET_BYTES}-byte safety limit"
        ));
    }
    let managed_payload_tokens = count_tokens(&packet)?;
    require_budget(managed_payload_tokens, inputs.max_tokens)?;
    let packet_hash = digest(&packet);
    let manifest = build_manifest_for(
        review_delta_id.clone(),
        plan,
        &inputs,
        packet_hash.clone(),
        managed_payload_tokens,
        contract,
    );
    let mut manifest_bytes =
        serde_json::to_vec_pretty(&manifest).expect("closed review delta manifest serializes");
    manifest_bytes.push(b'\n');
    let manifest_hash = digest(&manifest_bytes);
    let suffix = review_delta_id
        .strip_prefix("sha256:")
        .expect("generated review delta ID has a sha256 prefix");
    let output_directory = repository
        .join(".local-exclude/methexis/slice-review-deltas")
        .join(suffix);
    let status = storage::publish(&output_directory, &packet, &manifest_bytes, || {
        final_revalidate(
            &repository,
            &request,
            &inputs,
            contract,
            verify_affected_identity,
        )
    })?;

    let result = ResultRecord {
        schema: v1alpha1::RESULT_SCHEMA,
        ok: true,
        operation: "build_slice_review_delta",
        status,
        review_delta_id,
        prior_review_id: inputs.prior.review_id.clone(),
        prior_candidate_commit: inputs.prior.candidate_commit.clone(),
        replacement_candidate_commit: inputs.replacement_candidate.clone(),
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
    };
    let mut output = serde_json::to_vec(&result).expect("closed result serializes");
    output.push(b'\n');
    io::Write::write_all(&mut io::stdout().lock(), &output)
        .map_err(|error| format!("cannot write review delta result: {error}"))
}

fn normalize_request_schema(request: &mut Request) -> bool {
    if request.schema == REQUEST_SCHEMA_V1_ALPHA2 {
        request.schema = v1alpha1::REQUEST_SCHEMA.to_owned();
        true
    } else {
        false
    }
}

fn final_revalidate(
    repository: &Path,
    request: &Request,
    inputs: &Inputs,
    contract: WireContract,
    verify_affected_identity: bool,
) -> Result<(), String> {
    trusted_ensure_clean(repository, "returning a review delta")?;
    let bound = slice_contract::trusted_bound_slice(repository)?;
    require_expected_branch(repository, &bound.base_ref, &bound.slice)?;
    slice_contract::trusted_check_bound_scope(repository)?;
    if trusted_resolve_commit(repository, "HEAD")? != inputs.replacement_candidate {
        return Err("replacement candidate changed during delta construction".to_owned());
    }
    let verified = verify_chain_head(
        repository,
        &resolve_input_path(repository, &inputs.prior_manifest.path),
        &inputs.prior_manifest.hash,
        &mut BTreeSet::new(),
        0,
    )?;
    if verified.review_id != inputs.prior.review_id
        || verified.packet_hash != inputs.prior.packet_hash
        || verified.candidate_commit != inputs.prior.candidate_commit
        || verified.trusted_commit != inputs.prior.trusted_commit
    {
        return Err("prior published review identity changed during delta construction".to_owned());
    }
    if capture_delta(
        repository,
        &inputs.prior.candidate_commit,
        &inputs.replacement_candidate,
    )? != inputs.delta.bytes
    {
        return Err("prior-to-replacement delta changed during construction".to_owned());
    }
    require_current_file(
        &resolve_input_path(repository, &inputs.request.path),
        &inputs.request,
        "review delta request",
    )?;
    require_current_file(
        &resolve_input_path(repository, &inputs.prior_manifest.path),
        &inputs.prior_manifest,
        "prior review manifest",
    )?;
    require_current_packet(
        &resolve_input_path(repository, &inputs.prior_packet.path),
        &inputs.prior_packet,
        "prior review packet",
    )?;
    require_current_file(
        Path::new(&inputs.prior_findings.path),
        &inputs.prior_findings,
        "prior review findings",
    )?;
    require_exact_finding_set(&inputs.prior_findings, &inputs.findings)?;
    require_current_file(
        Path::new(&inputs.slice_contract.path),
        &inputs.slice_contract,
        "Slice contract",
    )?;
    if bound.base != inputs.prior.base_commit
        || bound.contract_id != inputs.slice_contract.hash
        || bound.contract_path != Path::new(&inputs.slice_contract.path)
    {
        return Err("bound Slice contract identity changed during delta construction".to_owned());
    }
    let (reused, affected) = capture_validation(
        repository,
        &inputs.prior,
        &inputs.replacement_candidate,
        verify_affected_identity,
        &request.reused_validation_evidence,
        &request.affected_validation_evidence,
    )?;
    require_named_captures(&reused, &inputs.reused_validation)?;
    require_named_captures(&affected, &inputs.affected_validation)?;
    validate_transition(
        TransitionContext::new(repository, contract.affected_path_policy),
        &inputs.prior,
        &inputs.replacement_candidate,
        &inputs.delta,
        &inputs.findings,
        &reused,
        &affected,
    )?;
    if delivery_profile_bytes_for(contract) != inputs.delivery_profile_bytes {
        return Err("delivery profile bytes changed during delta construction".to_owned());
    }
    Ok(())
}
