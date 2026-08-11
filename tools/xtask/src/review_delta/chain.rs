use std::{collections::BTreeSet, path::Path};

use super::{
    Inputs, MAX_INPUT_BYTES, MAX_PACKET_BYTES, WireContract,
    capture::{
        capture_file, capture_packet, capture_published, captured, require_exact_hash, require_hash,
    },
    evidence::{
        TransitionContext, add_evidence_size, capture_named_artifacts, require_exact_finding_set,
        validate_findings_artifact, validate_transition,
    },
    git_state::capture_delta,
    model::{Manifest, TOKENIZER_COMPILER, TOKENIZER_PROFILE},
    render::{
        build_manifest_for, build_plan_for, count_tokens, delivery_profile_bytes_for,
        delivery_profile_for, render_packet,
    },
    v1, v1alpha1,
};
use crate::{
    bounded_file,
    review_packet::{self, VerifiedReview},
    review_protocol::{artifact, digest, domain_digest, relative, resolve_input_path},
};

pub(super) fn verify_chain_head(
    repository: &Path,
    manifest_path: &Path,
    expected_hash: &str,
    seen: &mut BTreeSet<String>,
    depth: usize,
) -> Result<VerifiedReview, String> {
    verify_chain_head_with(
        repository,
        manifest_path,
        expected_hash,
        seen,
        depth,
        &review_packet::verify_published,
    )
}

pub(super) fn verify_chain_head_with(
    repository: &Path,
    manifest_path: &Path,
    expected_hash: &str,
    seen: &mut BTreeSet<String>,
    depth: usize,
    verify_original: &dyn Fn(&Path, &Path, &str) -> Result<VerifiedReview, String>,
) -> Result<VerifiedReview, String> {
    if depth >= 64 {
        return Err("review continuation chain exceeds the 64-hop safety limit".to_owned());
    }
    let manifest_bytes = bounded_file::read_regular(
        manifest_path,
        MAX_INPUT_BYTES,
        "published review-chain manifest",
    )?;
    require_exact_hash(
        expected_hash,
        &manifest_bytes,
        "published review-chain manifest",
    )?;
    if !seen.insert(expected_hash.to_owned()) {
        return Err("review continuation chain contains a cycle".to_owned());
    }
    let schema = serde_json::from_slice::<serde_json::Value>(&manifest_bytes)
        .map_err(|error| format!("invalid published review-chain manifest: {error}"))?
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "published review-chain manifest has no schema".to_owned())?
        .to_owned();
    if review_packet::is_original_manifest_schema(&schema) {
        return verify_original(repository, manifest_path, expected_hash);
    }
    let contract = contract_for_manifest_schema(&schema)?;

    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid published review delta manifest: {error}"))?;
    require_hash(&manifest.review_delta_id, "published ReviewDeltaId")?;
    if manifest.plan.schema != contract.plan_schema
        || manifest.plan.delivery_profile != delivery_profile_for(contract)
        || manifest.plan.tokenizer_profile != TOKENIZER_PROFILE
        || manifest.plan.tokenizer_compiler != TOKENIZER_COMPILER
        || manifest.packet.path != "packet.md"
        || manifest.packet.max_managed_payload_tokens != manifest.plan.max_managed_payload_tokens
    {
        return Err("published review delta manifest uses an unsupported contract".to_owned());
    }
    let expected_path = repository
        .join(".local-exclude/methexis/slice-review-deltas")
        .join(
            manifest
                .review_delta_id
                .strip_prefix("sha256:")
                .ok_or_else(|| "published ReviewDeltaId is not a SHA-256 identity".to_owned())?,
        )
        .join("manifest.json");
    if std::fs::canonicalize(manifest_path)
        .map_err(|error| format!("cannot resolve published delta manifest: {error}"))?
        != std::fs::canonicalize(&expected_path)
            .map_err(|error| format!("cannot resolve published ReviewDeltaId directory: {error}"))?
    {
        return Err("manifest is not the exact published ReviewDeltaId artifact".to_owned());
    }

    let prior_path = resolve_input_path(repository, &manifest.inputs.prior_manifest.path);
    let prior = verify_chain_head_with(
        repository,
        &prior_path,
        &manifest.inputs.prior_manifest.hash,
        seen,
        depth + 1,
        verify_original,
    )?;
    let prior_manifest = capture_published(
        repository,
        &prior_path,
        "prior review-chain manifest",
        MAX_INPUT_BYTES,
    )?;
    let prior_packet = capture_published(
        repository,
        &resolve_input_path(repository, &manifest.inputs.prior_packet.path),
        "prior review-chain packet",
        MAX_PACKET_BYTES,
    )?;
    if artifact(&prior_manifest) != manifest.inputs.prior_manifest
        || artifact(&prior_packet) != manifest.inputs.prior_packet
        || prior.packet_hash != prior_packet.hash
    {
        return Err("published review delta prior artifact identities differ".to_owned());
    }
    let prior_findings = capture_file(
        &resolve_input_path(repository, &manifest.inputs.prior_findings.path),
        "prior review findings",
    )?;
    if artifact(&prior_findings) != manifest.inputs.prior_findings {
        return Err("published review delta prior findings identity differs".to_owned());
    }
    validate_findings_artifact(&prior_findings, &prior)?;
    require_exact_finding_set(&prior_findings, &manifest.plan.finding_dispositions)?;

    let slice_contract = capture_file(
        &resolve_input_path(repository, &manifest.inputs.slice_contract.path),
        "Slice contract",
    )?;
    if artifact(&slice_contract) != manifest.inputs.slice_contract
        || slice_contract.path != prior.slice_contract_path
        || slice_contract.hash != prior.slice_contract_hash
    {
        return Err("published review delta Slice contract identity differs".to_owned());
    }
    let reused_validation = capture_named_artifacts(
        repository,
        &manifest.inputs.reused_validation_evidence,
        "reused validation evidence",
    )?;
    let affected_validation = capture_named_artifacts(
        repository,
        &manifest.inputs.affected_validation_evidence,
        "affected validation evidence",
    )?;
    let mut aggregate = 0usize;
    for evidence in reused_validation.iter().chain(&affected_validation) {
        add_evidence_size(&mut aggregate, evidence)?;
    }
    let replacement_candidate = manifest.plan.replacement_candidate_commit.clone();
    let delta = captured(
        "git-delta.patch".to_owned(),
        capture_delta(repository, &prior.candidate_commit, &replacement_candidate)?,
    )?;
    validate_transition(
        TransitionContext::new(repository, contract.affected_path_policy),
        &prior,
        &replacement_candidate,
        &delta,
        &manifest.plan.finding_dispositions,
        &reused_validation,
        &affected_validation,
    )?;
    let inputs = Inputs {
        request: captured(
            "verified-chain-request".to_owned(),
            b"verified chain".to_vec(),
        )?,
        prior_manifest,
        prior_packet,
        prior_findings,
        prior,
        replacement_candidate,
        delta,
        slice_contract,
        findings: manifest.plan.finding_dispositions.clone(),
        reused_validation,
        affected_validation,
        delivery_profile_bytes: delivery_profile_bytes_for(contract),
        max_tokens: manifest.plan.max_managed_payload_tokens,
    };
    let plan = build_plan_for(&inputs, contract);
    if plan != manifest.plan {
        return Err("published review delta plan does not reproduce from its inputs".to_owned());
    }
    let review_delta_id = domain_digest(
        contract.review_id_domain,
        &serde_json::to_vec(&plan).expect("review delta plan serializes"),
    );
    if review_delta_id != manifest.review_delta_id {
        return Err("published ReviewDeltaId does not match its canonical plan".to_owned());
    }
    let packet_path = expected_path
        .parent()
        .expect("published delta manifest has a parent")
        .join("packet.md");
    let packet = capture_packet(&packet_path, "published review delta packet")?;
    let reproduced_packet = render_packet(&review_delta_id, &plan, &inputs)?;
    if packet.bytes != reproduced_packet || packet.hash != manifest.packet.hash {
        return Err("published review delta packet does not reproduce exactly".to_owned());
    }
    let tokens = count_tokens(&packet.bytes)?;
    if tokens != manifest.packet.managed_payload_tokens
        || tokens > manifest.packet.max_managed_payload_tokens
    {
        return Err("published review delta token record does not match exact bytes".to_owned());
    }
    let reproduced_manifest = build_manifest_for(
        review_delta_id.clone(),
        plan,
        &inputs,
        packet.hash.clone(),
        tokens,
        contract,
    );
    let mut reproduced_manifest_bytes =
        serde_json::to_vec_pretty(&reproduced_manifest).expect("review delta manifest serializes");
    reproduced_manifest_bytes.push(b'\n');
    if reproduced_manifest_bytes != manifest_bytes {
        return Err("published review delta manifest bytes are not canonical".to_owned());
    }

    Ok(VerifiedReview {
        review_id: review_delta_id,
        manifest_path: relative(repository, &expected_path),
        manifest_hash: digest(&manifest_bytes),
        packet_path: relative(repository, &packet_path),
        packet_hash: packet.hash,
        base_commit: inputs.prior.base_commit,
        candidate_commit: inputs.replacement_candidate,
        trusted_commit: inputs.prior.trusted_commit,
        slice_contract_path: inputs.slice_contract.path,
        slice_contract_hash: inputs.slice_contract.hash,
        validation_evidence: inputs
            .reused_validation
            .into_iter()
            .chain(inputs.affected_validation)
            .map(|evidence| review_packet::VerifiedEvidence {
                name: evidence.name,
                path: evidence.artifact.path,
                hash: evidence.artifact.hash,
            })
            .collect(),
        review_lenses: inputs.prior.review_lenses,
        review_questions: inputs.prior.review_questions,
    })
}

fn contract_for_manifest_schema(schema: &str) -> Result<WireContract, String> {
    match schema {
        v1::MANIFEST_SCHEMA => Ok(v1::contract()),
        v1alpha1::MANIFEST_SCHEMA => Ok(v1alpha1::contract()),
        _ => Err(format!(
            "unsupported review-chain manifest schema `{schema}`"
        )),
    }
}
