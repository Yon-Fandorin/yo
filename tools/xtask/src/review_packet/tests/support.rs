use std::path::Path;

use super::super::{
    MAX_REQUEST_BYTES, REVIEW_ID_DOMAIN,
    canonical::{
        build_manifest, build_plan, delivery_profile_bytes_for_id, delivery_profile_v1_bytes,
    },
    capture::{
        ContextCapture, Inputs, ProspectiveCapture, capture_authorities, capture_diff,
        capture_validation, captured,
    },
    model::{
        CheckpointIdentity, ContextResult, DELIVERY_PROFILE_V1_ALPHA1, DELIVERY_PROFILE_V1_ALPHA2,
        DELIVERY_PROFILE_V1_ALPHA3, EvidenceRequest, Manifest, SemanticInput,
    },
    render::{count_tokens, render_packet_with_metadata},
    storage,
    verifier::{VerifiedEvidence, VerifiedReview, verify_canonical_artifacts},
};
use crate::{
    bounded_file,
    review_protocol::{NamedCaptured, artifact, digest, domain_digest, relative},
};

pub(crate) fn publish_original(
    repository: &Path,
    base_commit: &str,
    candidate_commit: &str,
    trusted_commit: &str,
    contract_path: &Path,
    validation_path: &Path,
) -> VerifiedReview {
    publish_original_with_profile(
        repository,
        base_commit,
        candidate_commit,
        trusted_commit,
        contract_path,
        validation_path,
        None,
    )
}

pub(crate) fn publish_original_v1_alpha1(
    repository: &Path,
    base_commit: &str,
    candidate_commit: &str,
    trusted_commit: &str,
    contract_path: &Path,
    validation_path: &Path,
) -> VerifiedReview {
    publish_original_with_profile(
        repository,
        base_commit,
        candidate_commit,
        trusted_commit,
        contract_path,
        validation_path,
        Some(DELIVERY_PROFILE_V1_ALPHA1),
    )
}

pub(crate) fn publish_original_v1_alpha2(
    repository: &Path,
    base_commit: &str,
    candidate_commit: &str,
    trusted_commit: &str,
    contract_path: &Path,
    validation_path: &Path,
) -> VerifiedReview {
    publish_original_with_profile(
        repository,
        base_commit,
        candidate_commit,
        trusted_commit,
        contract_path,
        validation_path,
        Some(DELIVERY_PROFILE_V1_ALPHA2),
    )
}

#[allow(clippy::too_many_arguments)]
fn publish_original_with_profile(
    repository: &Path,
    base_commit: &str,
    candidate_commit: &str,
    trusted_commit: &str,
    contract_path: &Path,
    validation_path: &Path,
    experimental_profile: Option<&str>,
) -> VerifiedReview {
    let mut inputs = sample_inputs("producer-shaped-validation");
    if let Some(profile) = experimental_profile {
        inputs.delivery_profile_bytes = delivery_profile_bytes_for_id(profile).unwrap();
    }
    inputs.base_commit = base_commit.to_owned();
    inputs.candidate_commit = candidate_commit.to_owned();
    inputs.diff = captured(
        "git-diff.patch".to_owned(),
        capture_diff(repository, base_commit, candidate_commit).unwrap(),
    )
    .unwrap();
    inputs.context.result.trusted_commit = trusted_commit.to_owned();
    inputs.authorities =
        capture_authorities(repository, candidate_commit, &["owned.txt".to_owned()]).unwrap();
    let contract_bytes =
        bounded_file::read_regular(contract_path, MAX_REQUEST_BYTES, "Slice review contract")
            .unwrap();
    inputs.slice_contract =
        captured(contract_path.to_string_lossy().into_owned(), contract_bytes).unwrap();
    inputs.validation = capture_validation(
        repository,
        candidate_commit,
        &[EvidenceRequest {
            name: "baseline".to_owned(),
            path: validation_path.to_string_lossy().into_owned(),
        }],
    )
    .unwrap();

    let plan = build_plan(&inputs);
    let plan_bytes = serde_json::to_vec(&plan).unwrap();
    let review_id = domain_digest(REVIEW_ID_DOMAIN, &plan_bytes);
    let rendered = render_packet_with_metadata(&review_id, &plan, &inputs).unwrap();
    let manifest = build_manifest(
        review_id,
        plan,
        &inputs,
        digest(&rendered.bytes),
        count_tokens(&rendered.bytes).unwrap(),
        rendered.input_prefix,
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    manifest_bytes.push(b'\n');
    let directory = repository
        .join(".local-exclude/methexis/slice-reviews")
        .join(manifest.review_id.strip_prefix("sha256:").unwrap());
    assert_eq!(
        storage::publish(&directory, &rendered.bytes, &manifest_bytes, || Ok(())).unwrap(),
        "created"
    );

    let manifest_path = directory.join("manifest.json");
    let packet_path = directory.join("packet.md");
    let published_manifest_bytes = std::fs::read(&manifest_path).unwrap();
    let published_packet_bytes = std::fs::read(&packet_path).unwrap();
    assert_eq!(published_manifest_bytes, manifest_bytes);
    assert_eq!(published_packet_bytes, rendered.bytes);
    let published_manifest: Manifest = serde_json::from_slice(&published_manifest_bytes).unwrap();
    verify_canonical_artifacts(
        &published_manifest,
        &published_manifest_bytes,
        &published_packet_bytes,
        &inputs,
    )
    .unwrap();

    VerifiedReview {
        review_id: published_manifest.review_id,
        manifest_path: relative(repository, &manifest_path),
        manifest_hash: digest(&published_manifest_bytes),
        packet_path: relative(repository, &packet_path),
        packet_hash: published_manifest.packet.hash,
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
    }
}

pub(super) fn sample_inputs(validation_path: &str) -> Inputs {
    let context_request = captured(
        "context-request.json".to_owned(),
        b"context request".to_vec(),
    )
    .unwrap();
    let context = captured("context.md".to_owned(), b"context".to_vec()).unwrap();
    let manifest = captured(
        "context-manifest.json".to_owned(),
        b"context manifest".to_vec(),
    )
    .unwrap();
    Inputs {
        base_commit: "0000000000000000000000000000000000000000".to_owned(),
        candidate_commit: "1111111111111111111111111111111111111111".to_owned(),
        diff: captured("git-diff.patch".to_owned(), b"diff".to_vec()).unwrap(),
        context: ContextCapture {
            result: ContextResult {
                schema: "methexis.context-result/v1alpha1".to_owned(),
                ok: true,
                operation: "resolve_context".to_owned(),
                authority: "trusted_integration".to_owned(),
                trusted_commit: "2222222222222222222222222222222222222222".to_owned(),
                build_id: "sha256:build".to_owned(),
                context: artifact(&context),
                manifest: artifact(&manifest),
                checkpoint: None,
                activation_request: None,
                predecessor_active_record_hash: None,
                proposed_active_record_hash: None,
            },
            request: context_request,
            context,
            manifest,
            active_checkpoint: CheckpointIdentity {
                id: "sha256:checkpoint".to_owned(),
                hash: "sha256:checkpoint-hash".to_owned(),
                authority_basis_commit: "3333333333333333333333333333333333333333".to_owned(),
            },
            included_ids: vec!["methexis.review.bounded-packet".to_owned()],
        },
        prospective: None,
        authorities: vec![captured("CONTRIBUTING.md".to_owned(), b"authority".to_vec()).unwrap()],
        slice_contract: captured("slice-contract.json".to_owned(), b"contract".to_vec()).unwrap(),
        validation: vec![NamedCaptured {
            name: "validation".to_owned(),
            artifact: captured(validation_path.to_owned(), b"passed".to_vec()).unwrap(),
        }],
        lenses: vec!["fresh-context".to_owned()],
        questions: vec!["Is it correct?".to_owned()],
        required_knowledge_ids: vec!["methexis.review.bounded-packet".to_owned()],
        delivery_profile_bytes: delivery_profile_v1_bytes(),
        max_tokens: 10_000,
    }
}

pub(super) fn sample_inputs_v1_alpha1(validation_path: &str) -> Inputs {
    let mut inputs = sample_inputs(validation_path);
    inputs.delivery_profile_bytes =
        delivery_profile_bytes_for_id(DELIVERY_PROFILE_V1_ALPHA1).unwrap();
    inputs
}

pub(super) fn sample_inputs_v1_alpha2(validation_path: &str) -> Inputs {
    let mut inputs = sample_inputs(validation_path);
    inputs.delivery_profile_bytes =
        delivery_profile_bytes_for_id(DELIVERY_PROFILE_V1_ALPHA2).unwrap();
    inputs
}

pub(super) fn sample_inputs_v1_alpha3(validation_path: &str) -> Inputs {
    let mut inputs = sample_inputs(validation_path);
    let activation_request = captured(
        "/worktree/.local-exclude/activation.json".to_owned(),
        b"{\"schema\":\"methexis.activation-request/v1alpha1\"}\n".to_vec(),
    )
    .unwrap();
    let proposed_checkpoint = captured(
        "methexis/checkpoints/proposed.yaml".to_owned(),
        b"proposed checkpoint\n".to_vec(),
    )
    .unwrap();
    let proposed_active_record = captured(
        "methexis/active-checkpoint.yaml".to_owned(),
        b"proposed active record\n".to_vec(),
    )
    .unwrap();
    let checkpoint = CheckpointIdentity {
        id: "sha256:prospective-checkpoint".to_owned(),
        hash: proposed_checkpoint.hash.clone(),
        authority_basis_commit: inputs.context.result.trusted_commit.clone(),
    };
    inputs.context.result.schema = "methexis.activation-review-context-result/v1alpha1".to_owned();
    inputs.context.result.operation = "resolve_activation_review_context".to_owned();
    inputs.context.result.authority = "prospective".to_owned();
    inputs.context.result.checkpoint = Some(checkpoint.clone());
    inputs.context.result.activation_request = Some(SemanticInput {
        path: activation_request.path.clone(),
        hash: activation_request.hash.clone(),
    });
    inputs.context.result.predecessor_active_record_hash =
        Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned());
    inputs.context.result.proposed_active_record_hash = Some(proposed_active_record.hash.clone());
    inputs.context.active_checkpoint = checkpoint;
    inputs.prospective = Some(ProspectiveCapture {
        activation_request,
        proposed_checkpoint,
        proposed_active_record,
        predecessor_active_record_hash: inputs
            .context
            .result
            .predecessor_active_record_hash
            .clone(),
    });
    inputs.delivery_profile_bytes =
        delivery_profile_bytes_for_id(DELIVERY_PROFILE_V1_ALPHA3).unwrap();
    inputs
}
