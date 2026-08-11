use std::path::Path;

use super::{
    MAX_INPUT_BYTES, MAX_PACKET_BYTES, REVIEW_ID_DOMAIN,
    canonical::{build_manifest, build_plan, delivery_profile},
    capture::{
        Inputs, capture_authorities, capture_context, capture_diff, capture_validation, captured,
        require_hash,
    },
    model::{EvidenceRequest, Manifest, PLAN_SCHEMA, TOKENIZER_COMPILER, TOKENIZER_PROFILE},
    render::{count_tokens, render_packet},
    trusted_git::{trusted_repository_root, trusted_resolve_commit},
};
use crate::review_protocol::{Captured, digest, domain_digest, relative, resolve_input_path};

#[derive(Clone, Debug)]
pub(crate) struct VerifiedEvidence {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) hash: String,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedReview {
    pub(crate) review_id: String,
    pub(crate) manifest_path: String,
    pub(crate) manifest_hash: String,
    pub(crate) packet_path: String,
    pub(crate) packet_hash: String,
    pub(crate) base_commit: String,
    pub(crate) candidate_commit: String,
    pub(crate) trusted_commit: String,
    pub(crate) slice_contract_path: String,
    pub(crate) slice_contract_hash: String,
    pub(crate) validation_evidence: Vec<VerifiedEvidence>,
    pub(crate) review_lenses: Vec<String>,
    pub(crate) review_questions: Vec<String>,
}

pub(crate) fn verify_published(
    repository: &Path,
    manifest_path: &Path,
    expected_manifest_hash: &str,
) -> Result<VerifiedReview, String> {
    let repository = trusted_repository_root(repository)?;
    let manifest_bytes = crate::bounded_file::read_regular(
        manifest_path,
        MAX_INPUT_BYTES,
        "published Slice review manifest",
    )?;
    require_hash(
        expected_manifest_hash,
        &manifest_bytes,
        "published Slice review manifest",
    )?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("invalid published Slice review manifest: {error}"))?;
    require_supported_manifest(&manifest)?;
    let plan_bytes = serde_json::to_vec(&manifest.plan).expect("review plan serializes");
    let reproduced_id = domain_digest(REVIEW_ID_DOMAIN, &plan_bytes);
    if reproduced_id != manifest.review_id {
        return Err("published ReviewId does not match its canonical plan".to_owned());
    }
    let suffix = manifest
        .review_id
        .strip_prefix("sha256:")
        .expect("reproduced ReviewId has a prefix");
    let expected_path = repository
        .join(".local-exclude/methexis/slice-reviews")
        .join(suffix)
        .join("manifest.json");
    if std::fs::canonicalize(manifest_path)
        .map_err(|error| format!("cannot resolve published review manifest: {error}"))?
        != std::fs::canonicalize(&expected_path)
            .map_err(|error| format!("cannot resolve published ReviewId directory: {error}"))?
    {
        return Err("manifest is not the exact published ReviewId artifact".to_owned());
    }

    if trusted_resolve_commit(&repository, "refs/heads/develop")? != manifest.plan.trusted_commit {
        return Err("trusted integration changed since the published review".to_owned());
    }
    let context_request_path =
        resolve_input_path(&repository, &manifest.inputs.context_request.path);
    let context = capture_context(&repository, &context_request_path)?;
    let authorities = capture_authorities(
        &repository,
        &manifest.plan.candidate_commit,
        &manifest
            .plan
            .repository_authorities
            .iter()
            .map(|input| input.path.clone())
            .collect::<Vec<_>>(),
    )?;
    let validation_requests = manifest
        .inputs
        .validation_evidence
        .iter()
        .map(|input| EvidenceRequest {
            name: input.name.clone(),
            path: input.artifact.path.clone(),
        })
        .collect::<Vec<_>>();
    let validation = capture_validation(
        &repository,
        &manifest.plan.candidate_commit,
        &validation_requests,
    )?;
    let contract_path = resolve_input_path(&repository, &manifest.inputs.slice_contract.path);
    let contract_bytes = crate::bounded_file::read_regular(
        &contract_path,
        super::MAX_REQUEST_BYTES,
        "Slice review contract",
    )?;
    let slice_contract = Captured {
        path: contract_path.to_string_lossy().into_owned(),
        hash: digest(&contract_bytes),
        bytes: contract_bytes,
    };
    let inputs = Inputs {
        base_commit: manifest.plan.base_commit.clone(),
        candidate_commit: manifest.plan.candidate_commit.clone(),
        diff: captured(
            "git-diff.patch".to_owned(),
            capture_diff(
                &repository,
                &manifest.plan.base_commit,
                &manifest.plan.candidate_commit,
            )?,
        )?,
        context,
        authorities,
        slice_contract,
        validation,
        lenses: manifest.plan.review_lenses.clone(),
        questions: manifest.plan.review_questions.clone(),
        required_knowledge_ids: manifest.plan.required_knowledge_ids.clone(),
        delivery_profile_bytes: super::canonical::delivery_profile_bytes(),
        max_tokens: manifest.plan.max_managed_payload_tokens,
    };
    let packet_path = expected_path
        .parent()
        .expect("published manifest path has a parent")
        .join("packet.md");
    let packet_bytes = crate::bounded_file::read_regular(
        &packet_path,
        MAX_PACKET_BYTES,
        "published review packet",
    )?;
    verify_canonical_artifacts(&manifest, &manifest_bytes, &packet_bytes, &inputs)?;

    Ok(VerifiedReview {
        review_id: manifest.review_id,
        manifest_path: relative(&repository, &expected_path),
        manifest_hash: digest(&manifest_bytes),
        packet_path: relative(&repository, &packet_path),
        packet_hash: manifest.packet.hash,
        base_commit: inputs.base_commit,
        candidate_commit: inputs.candidate_commit,
        trusted_commit: inputs.context.result.trusted_commit,
        slice_contract_path: inputs.slice_contract.path,
        slice_contract_hash: inputs.slice_contract.hash,
        validation_evidence: inputs
            .validation
            .into_iter()
            .map(|input| VerifiedEvidence {
                name: input.name,
                path: input.artifact.path,
                hash: input.artifact.hash,
            })
            .collect(),
        review_lenses: inputs.lenses,
        review_questions: inputs.questions,
    })
}

pub(super) fn verify_canonical_artifacts(
    manifest: &Manifest,
    manifest_bytes: &[u8],
    packet_bytes: &[u8],
    inputs: &Inputs,
) -> Result<(), String> {
    require_supported_manifest(manifest)?;
    let reproduced_plan = build_plan(inputs);
    if reproduced_plan != manifest.plan {
        return Err("published review plan does not match its complete captured inputs".to_owned());
    }
    let plan_bytes = serde_json::to_vec(&reproduced_plan).expect("review plan serializes");
    if domain_digest(REVIEW_ID_DOMAIN, &plan_bytes) != manifest.review_id {
        return Err("published ReviewId does not match its canonical plan".to_owned());
    }
    let reproduced_packet = render_packet(&manifest.review_id, &reproduced_plan, inputs)?;
    if packet_bytes != reproduced_packet {
        return Err(
            "published review packet does not reproduce from its complete inputs".to_owned(),
        );
    }
    require_hash(
        &manifest.packet.hash,
        packet_bytes,
        "published review packet",
    )?;
    let tokens = count_tokens(packet_bytes)?;
    if tokens != manifest.packet.managed_payload_tokens
        || tokens > manifest.packet.max_managed_payload_tokens
    {
        return Err(
            "published review packet token record does not match its exact bytes".to_owned(),
        );
    }
    let reproduced_manifest = build_manifest(
        manifest.review_id.clone(),
        reproduced_plan,
        inputs,
        manifest.packet.hash.clone(),
        tokens,
    );
    let mut reproduced_manifest_bytes = serde_json::to_vec_pretty(&reproduced_manifest)
        .expect("published review manifest serializes");
    reproduced_manifest_bytes.push(b'\n');
    if reproduced_manifest_bytes != manifest_bytes {
        return Err("published review manifest bytes are not canonical".to_owned());
    }
    Ok(())
}

fn require_supported_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema != super::model::MANIFEST_SCHEMA
        || manifest.plan.schema != PLAN_SCHEMA
        || manifest.plan.delivery_profile != delivery_profile()
        || manifest.plan.tokenizer_profile != TOKENIZER_PROFILE
        || manifest.plan.tokenizer_compiler != TOKENIZER_COMPILER
        || manifest.packet.path != "packet.md"
        || manifest.packet.max_managed_payload_tokens != manifest.plan.max_managed_payload_tokens
    {
        return Err("published Slice review manifest uses an unsupported contract".to_owned());
    }
    Ok(())
}
