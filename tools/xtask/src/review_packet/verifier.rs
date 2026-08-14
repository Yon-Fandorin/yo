use std::path::Path;

use super::{
    MAX_INPUT_BYTES, MAX_PACKET_BYTES,
    bootstrap::require_prospective_activation_boundary,
    canonical::{
        build_manifest, build_plan, delivery_profile_bytes_for_id, delivery_profile_for_id,
    },
    capture::{
        Inputs, capture_authorities, capture_context, capture_context_request, capture_diff,
        capture_prospective_context_with_request, capture_validation, captured, require_hash,
    },
    model::{
        DELIVERY_PROFILE_V1, DELIVERY_PROFILE_V1_ALPHA1, DELIVERY_PROFILE_V1_ALPHA2,
        DELIVERY_PROFILE_V1_ALPHA3, EvidenceRequest, MANIFEST_SCHEMA_V1, MANIFEST_SCHEMA_V1_ALPHA1,
        MANIFEST_SCHEMA_V1_ALPHA2, MANIFEST_SCHEMA_V1_ALPHA3, Manifest, PLAN_SCHEMA,
        PLAN_SCHEMA_V1_ALPHA3, TOKENIZER_COMPILER, TOKENIZER_PROFILE,
    },
    render::{count_tokens, render_packet_with_metadata},
    trusted_git::{trusted_git_succeeds, trusted_repository_root, trusted_resolve_commit},
};
use crate::review_protocol::{
    Captured, digest, domain_digest, relative, require_commit, resolve_input_path,
};

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
    require_manifest_revisions(&manifest)?;
    let plan_bytes = serde_json::to_vec(&manifest.plan).expect("review plan serializes");
    let reproduced_id = domain_digest(
        review_id_domain(&manifest.plan.delivery_profile.id),
        &plan_bytes,
    );
    if reproduced_id != manifest.review_id {
        return Err("published ReviewId does not match its canonical plan".to_owned());
    }
    let repository = trusted_repository_root(repository)?;
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

    require_base_candidate_provenance(
        &repository,
        &manifest.plan.base_commit,
        &manifest.plan.candidate_commit,
    )?;

    if trusted_resolve_commit(&repository, "refs/heads/develop")? != manifest.plan.trusted_commit {
        return Err("trusted integration changed since the published review".to_owned());
    }
    let context_request_path =
        resolve_input_path(&repository, &manifest.inputs.context_request.path);
    let (context, prospective) = if let Some(proposal) = &manifest.inputs.prospective_activation {
        let activation_request_path =
            resolve_input_path(&repository, &proposal.activation_request.path);
        let activation_request_bytes = crate::bounded_file::read_regular(
            &activation_request_path,
            super::MAX_REQUEST_BYTES,
            "prospective activation request",
        )?;
        require_hash(
            &proposal.activation_request.hash,
            &activation_request_bytes,
            "prospective activation request",
        )?;
        let activation_request = captured(
            activation_request_path.to_string_lossy().into_owned(),
            activation_request_bytes,
        )?;
        require_prospective_activation_boundary(
            &repository,
            &manifest.plan.trusted_commit,
            &manifest.plan.candidate_commit,
            &activation_request,
        )?;
        let context_request = capture_context_request(&repository, &context_request_path)?;
        let (context, prospective) = capture_prospective_context_with_request(
            &repository,
            &manifest.plan.candidate_commit,
            &activation_request_path,
            activation_request,
            &context_request_path,
            context_request,
        )?;
        (context, Some(prospective))
    } else {
        (capture_context(&repository, &context_request_path)?, None)
    };
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
        prospective,
        authorities,
        slice_contract,
        validation,
        lenses: manifest.plan.review_lenses.clone(),
        questions: manifest.plan.review_questions.clone(),
        required_knowledge_ids: manifest.plan.required_knowledge_ids.clone(),
        delivery_profile_bytes: delivery_profile_bytes_for_id(&manifest.plan.delivery_profile.id)?,
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

fn require_manifest_revisions(manifest: &Manifest) -> Result<(), String> {
    let checkpoint = manifest
        .plan
        .active_checkpoint
        .as_ref()
        .or(manifest.plan.prospective_checkpoint.as_ref())
        .ok_or_else(|| "published review plan omits its Checkpoint identity".to_owned())?;
    for (value, label) in [
        (manifest.plan.base_commit.as_str(), "published review base"),
        (
            manifest.plan.candidate_commit.as_str(),
            "published review candidate",
        ),
        (
            manifest.plan.trusted_commit.as_str(),
            "published review trusted integration",
        ),
        (
            checkpoint.authority_basis_commit.as_str(),
            "published review Checkpoint authority basis",
        ),
    ] {
        require_commit(value, label)?;
    }
    Ok(())
}

fn require_exact_commit(repository: &Path, revision: &str, label: &str) -> Result<(), String> {
    let resolved = trusted_resolve_commit(repository, revision)
        .map_err(|error| format!("cannot resolve published review {label}: {error}"))?;
    if resolved == revision {
        Ok(())
    } else {
        Err(format!(
            "published review {label} does not name the exact commit object"
        ))
    }
}

pub(super) fn require_base_candidate_provenance(
    repository: &Path,
    base: &str,
    candidate: &str,
) -> Result<(), String> {
    require_exact_commit(repository, base, "base revision")?;
    require_exact_commit(repository, candidate, "candidate revision")?;
    if trusted_git_succeeds(
        repository,
        &["merge-base", "--is-ancestor", base, candidate],
    )? {
        Ok(())
    } else {
        Err("published review base is not an ancestor of its candidate".to_owned())
    }
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
    if domain_digest(
        review_id_domain(&manifest.plan.delivery_profile.id),
        &plan_bytes,
    ) != manifest.review_id
    {
        return Err("published ReviewId does not match its canonical plan".to_owned());
    }
    let reproduced_packet =
        render_packet_with_metadata(&manifest.review_id, &reproduced_plan, inputs)?;
    if manifest.input_prefix != reproduced_packet.input_prefix {
        return Err(
            "published review input-prefix record does not match its exact packet bytes".to_owned(),
        );
    }
    if packet_bytes != reproduced_packet.bytes {
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
        reproduced_packet.input_prefix,
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
    let profile_id = manifest.plan.delivery_profile.id.as_str();
    let schema_and_prefix_match = matches!(
        (
            manifest.schema.as_str(),
            profile_id,
            manifest.input_prefix.is_some()
        ),
        (MANIFEST_SCHEMA_V1, DELIVERY_PROFILE_V1, false)
            | (MANIFEST_SCHEMA_V1_ALPHA1, DELIVERY_PROFILE_V1_ALPHA1, true)
            | (MANIFEST_SCHEMA_V1_ALPHA2, DELIVERY_PROFILE_V1_ALPHA2, true)
            | (MANIFEST_SCHEMA_V1_ALPHA3, DELIVERY_PROFILE_V1_ALPHA3, true)
    );
    let authority_shape_matches = if profile_id == DELIVERY_PROFILE_V1_ALPHA3 {
        manifest.plan.schema == PLAN_SCHEMA_V1_ALPHA3
            && manifest.plan.authority_mode.as_deref() == Some("prospective")
            && manifest.plan.active_checkpoint.is_none()
            && manifest.plan.prospective_checkpoint.is_some()
            && manifest.plan.prospective_activation.is_some()
            && manifest.inputs.prospective_activation.is_some()
    } else {
        manifest.plan.schema == PLAN_SCHEMA
            && manifest.plan.authority_mode.is_none()
            && manifest.plan.active_checkpoint.is_some()
            && manifest.plan.prospective_checkpoint.is_none()
            && manifest.plan.prospective_activation.is_none()
            && manifest.inputs.prospective_activation.is_none()
    };
    if !schema_and_prefix_match
        || !authority_shape_matches
        || manifest.plan.delivery_profile != delivery_profile_for_id(profile_id)?
        || manifest.plan.tokenizer_profile != TOKENIZER_PROFILE
        || manifest.plan.tokenizer_compiler != TOKENIZER_COMPILER
        || manifest.packet.path != "packet.md"
        || manifest.packet.max_managed_payload_tokens != manifest.plan.max_managed_payload_tokens
    {
        return Err("published Slice review manifest uses an unsupported contract".to_owned());
    }
    Ok(())
}

fn review_id_domain(profile: &str) -> &'static [u8] {
    if profile == DELIVERY_PROFILE_V1_ALPHA3 {
        super::REVIEW_ID_DOMAIN_V1_ALPHA3
    } else {
        super::REVIEW_ID_DOMAIN
    }
}
