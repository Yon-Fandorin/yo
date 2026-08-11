use std::path::Path;

use super::super::{
    MAX_REQUEST_BYTES, REVIEW_ID_DOMAIN,
    canonical::{build_manifest, build_plan, delivery_profile_bytes},
    capture::{
        ContextCapture, Inputs, capture_authorities, capture_diff, capture_validation, captured,
    },
    model::{CheckpointIdentity, ContextResult, EvidenceRequest, Manifest},
    render::{count_tokens, render_packet},
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
    let mut inputs = sample_inputs("producer-shaped-validation");
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
    let packet = render_packet(&review_id, &plan, &inputs).unwrap();
    let manifest = build_manifest(
        review_id,
        plan,
        &inputs,
        digest(&packet),
        count_tokens(&packet).unwrap(),
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    manifest_bytes.push(b'\n');
    let directory = repository
        .join(".local-exclude/methexis/slice-reviews")
        .join(manifest.review_id.strip_prefix("sha256:").unwrap());
    assert_eq!(
        storage::publish(&directory, &packet, &manifest_bytes, || Ok(())).unwrap(),
        "created"
    );

    let manifest_path = directory.join("manifest.json");
    let packet_path = directory.join("packet.md");
    let published_manifest_bytes = std::fs::read(&manifest_path).unwrap();
    let published_packet_bytes = std::fs::read(&packet_path).unwrap();
    assert_eq!(published_manifest_bytes, manifest_bytes);
    assert_eq!(published_packet_bytes, packet);
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
        authorities: vec![captured("CONTRIBUTING.md".to_owned(), b"authority".to_vec()).unwrap()],
        slice_contract: captured("slice-contract.json".to_owned(), b"contract".to_vec()).unwrap(),
        validation: vec![NamedCaptured {
            name: "validation".to_owned(),
            artifact: captured(validation_path.to_owned(), b"passed".to_vec()).unwrap(),
        }],
        lenses: vec!["fresh-context".to_owned()],
        questions: vec!["Is it correct?".to_owned()],
        required_knowledge_ids: vec!["methexis.review.bounded-packet".to_owned()],
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: 10_000,
    }
}
