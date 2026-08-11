use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::{
    super::{
        Inputs, MAX_INPUT_BYTES, MAX_PACKET_BYTES, REVIEW_DELTA_ID_DOMAIN,
        capture::{capture_file, capture_published, captured},
        chain::verify_chain_head_with,
        evidence::validate_transition,
        git_state::capture_delta,
        render::{build_manifest, build_plan, count_tokens, delivery_profile_bytes, render_packet},
    },
    support::finding,
};
use crate::{
    git,
    review_delta::model::{Manifest, PRIOR_FINDINGS_SCHEMA, PriorFinding, PriorFindings},
    review_packet::{VerifiedReview, storage},
    review_protocol::{
        Captured, NamedCaptured, digest, domain_digest, relative, resolve_input_path,
    },
};

fn repository_head(repository: &Path) -> String {
    git::output_in(repository, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned()
}

fn write_findings(path: &Path, review_id: &str, candidate: &str, finding_id: &str) -> Captured {
    let value = PriorFindings {
        schema: PRIOR_FINDINGS_SCHEMA.to_owned(),
        review_id: review_id.to_owned(),
        candidate_commit: candidate.to_owned(),
        findings: vec![PriorFinding {
            finding_id: finding_id.to_owned(),
            summary: "review finding".to_owned(),
        }],
    };
    let mut bytes = serde_json::to_vec_pretty(&value).unwrap();
    bytes.push(b'\n');
    std::fs::write(path, &bytes).unwrap();
    captured(path.to_string_lossy().into_owned(), bytes).unwrap()
}

fn publish_delta_fixture(repository: &Path, inputs: &Inputs) -> (PathBuf, String, &'static str) {
    let plan = build_plan(inputs);
    let id = domain_digest(REVIEW_DELTA_ID_DOMAIN, &serde_json::to_vec(&plan).unwrap());
    let packet = render_packet(&id, &plan, inputs).unwrap();
    let manifest = build_manifest(
        id.clone(),
        plan,
        inputs,
        digest(&packet),
        count_tokens(&packet).unwrap(),
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    manifest_bytes.push(b'\n');
    let directory = repository
        .join(".local-exclude/methexis/slice-review-deltas")
        .join(id.strip_prefix("sha256:").unwrap());
    let status = storage::publish(&directory, &packet, &manifest_bytes, || Ok(())).unwrap();
    (
        directory.join("manifest.json"),
        digest(&manifest_bytes),
        status,
    )
}

fn delta_inputs(
    repository: &Path,
    prior: VerifiedReview,
    replacement: &str,
    finding_id: &str,
    evidence_body: &[u8],
    evidence_suffix: &str,
) -> Inputs {
    let prior_manifest = capture_published(
        repository,
        &resolve_input_path(repository, &prior.manifest_path),
        "prior manifest",
        MAX_INPUT_BYTES,
    )
    .unwrap();
    let prior_packet = capture_published(
        repository,
        &resolve_input_path(repository, &prior.packet_path),
        "prior packet",
        MAX_PACKET_BYTES,
    )
    .unwrap();
    let findings_path = repository.join(format!(".local-exclude/findings-{evidence_suffix}.json"));
    let prior_findings = write_findings(
        &findings_path,
        &prior.review_id,
        &prior.candidate_commit,
        finding_id,
    );
    let evidence_path = repository.join(format!(".local-exclude/evidence-{evidence_suffix}.txt"));
    std::fs::write(&evidence_path, evidence_body).unwrap();
    let contract_path = resolve_input_path(repository, &prior.slice_contract_path);
    Inputs {
        request: captured("request.json".to_owned(), b"request".to_vec()).unwrap(),
        prior_manifest,
        prior_packet,
        prior_findings,
        delta: captured(
            "git-delta.patch".to_owned(),
            capture_delta(repository, &prior.candidate_commit, replacement).unwrap(),
        )
        .unwrap(),
        slice_contract: capture_file(&contract_path, "contract").unwrap(),
        findings: vec![finding(finding_id)],
        reused_validation: Vec::new(),
        affected_validation: vec![NamedCaptured {
            name: "baseline".to_owned(),
            artifact: capture_file(&evidence_path, "evidence").unwrap(),
        }],
        prior,
        replacement_candidate: replacement.to_owned(),
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: 20_000,
    }
}

// 실제 review-packet serialization/publication/canonical verification으로 원본을 만든 뒤 두 번의
// published delta를 재생해 중앙 verifier가 재귀 chain과 alias reuse를 수락하고
// canonical-but-ineligible evidence는 거부하는지 끝까지 확인한다.
#[test]
fn recursive_chain_verifier_replays_two_hops_and_rejects_ineligible_artifacts() {
    let repository = crate::test_support::TestRepository::new("review-delta-chain-e2e");
    repository.write(".gitignore", ".local-exclude/\n");
    repository.write("owned.txt", "base\n");
    repository.git(["add", ".gitignore", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    let base_commit = repository_head(&repository.path);
    repository.write("owned.txt", "candidate a\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate a"]);
    let candidate_a = repository_head(&repository.path);
    repository.git(["switch", "-c", "slice/direct/review-delta-chain"]);

    let contract_path = repository.write(".local-exclude/contract.json", "contract\n");
    let baseline_path = repository.write(
        ".local-exclude/evidence-a.txt",
        &format!("Candidate: {candidate_a}\n"),
    );
    let seed = crate::review_packet::tests::support::publish_original(
        &repository.path,
        &base_commit,
        &candidate_a,
        &candidate_a,
        &contract_path,
        &baseline_path,
    );
    let seed_manifest_path = resolve_input_path(&repository.path, &seed.manifest_path);
    let verify_seed = |_: &Path, path: &Path, expected: &str| {
        if std::fs::canonicalize(path).unwrap()
            == std::fs::canonicalize(&seed_manifest_path).unwrap()
            && expected == seed.manifest_hash
        {
            Ok(seed.clone())
        } else {
            Err("unexpected seed review".to_owned())
        }
    };

    repository.write("owned.txt", "candidate b\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate b"]);
    let candidate_b = repository_head(&repository.path);
    let first_inputs = delta_inputs(
        &repository.path,
        seed.clone(),
        &candidate_b,
        "F1",
        format!("Candidate: {candidate_b}\npassed\n").as_bytes(),
        "b",
    );
    validate_transition(
        &first_inputs.prior,
        &candidate_b,
        &first_inputs.delta,
        &first_inputs.findings,
        &first_inputs.reused_validation,
        &first_inputs.affected_validation,
    )
    .unwrap();
    let (first_manifest, first_hash, created) =
        publish_delta_fixture(&repository.path, &first_inputs);
    assert_eq!(created, "created");
    assert_eq!(
        publish_delta_fixture(&repository.path, &first_inputs).2,
        "reused"
    );
    let first = verify_chain_head_with(
        &repository.path,
        &first_manifest,
        &first_hash,
        &mut BTreeSet::new(),
        0,
        &verify_seed,
    )
    .unwrap();
    let first_manifest_value: Manifest =
        serde_json::from_slice(&std::fs::read(&first_manifest).unwrap()).unwrap();
    assert_eq!(
        first_manifest_value.plan.prior_candidate_commit,
        candidate_a
    );
    assert_eq!(
        first_manifest_value.plan.replacement_candidate_commit,
        candidate_b
    );
    assert_eq!(
        first_manifest_value.inputs.prior_manifest.path,
        seed.manifest_path
    );
    assert_eq!(
        first_manifest_value.inputs.prior_packet.path,
        seed.packet_path
    );
    assert_eq!(first.candidate_commit, candidate_b);
    assert_eq!(first.base_commit, base_commit);
    assert_eq!(first.trusted_commit, candidate_a);
    assert_eq!(first.slice_contract_path, contract_path.to_string_lossy());
    assert_eq!(first.validation_evidence.len(), 1);
    assert_eq!(first.validation_evidence[0].name, "baseline");
    assert!(
        first.validation_evidence[0]
            .path
            .ends_with("evidence-b.txt")
    );
    let first_manifest_relative = first.manifest_path.clone();
    let first_packet_relative = first.packet_path.clone();

    repository.write("owned.txt", "candidate c\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate c"]);
    let candidate_c = repository_head(&repository.path);
    let second_inputs = delta_inputs(
        &repository.path,
        first,
        &candidate_c,
        "F2",
        format!("Candidate: {candidate_c}\npassed\n").as_bytes(),
        "c",
    );
    let (second_manifest, second_hash, _) = publish_delta_fixture(&repository.path, &second_inputs);
    let alias = second_manifest.parent().unwrap().join("./manifest.json");
    let second = verify_chain_head_with(
        &repository.path,
        &alias,
        &second_hash,
        &mut BTreeSet::new(),
        0,
        &verify_seed,
    )
    .unwrap();
    assert_eq!(second.candidate_commit, candidate_c);
    assert_eq!(second.base_commit, base_commit);
    assert_eq!(second.trusted_commit, candidate_a);
    assert_eq!(second.slice_contract_path, contract_path.to_string_lossy());
    assert_eq!(second.validation_evidence.len(), 1);
    assert_eq!(second.validation_evidence[0].name, "baseline");
    assert!(
        second.validation_evidence[0]
            .path
            .ends_with("evidence-c.txt")
    );
    assert_eq!(
        second.manifest_path,
        relative(&repository.path, &second_manifest)
    );
    let second_manifest_value: Manifest =
        serde_json::from_slice(&std::fs::read(&second_manifest).unwrap()).unwrap();
    assert_eq!(
        second_manifest_value.plan.prior_candidate_commit,
        candidate_b
    );
    assert_eq!(
        second_manifest_value.plan.replacement_candidate_commit,
        candidate_c
    );
    assert_eq!(
        second_manifest_value.inputs.prior_manifest.path,
        first_manifest_relative
    );
    assert_eq!(
        second_manifest_value.inputs.prior_packet.path,
        first_packet_relative
    );
    assert_eq!(
        second_manifest_value.inputs.affected_validation_evidence[0]
            .artifact
            .path,
        second.validation_evidence[0].path
    );

    let invalid_inputs = delta_inputs(
        &repository.path,
        seed.clone(),
        &candidate_c,
        "F3",
        b"passed without candidate binding\n",
        "invalid",
    );
    let (invalid_manifest, invalid_hash, _) =
        publish_delta_fixture(&repository.path, &invalid_inputs);
    assert!(
        verify_chain_head_with(
            &repository.path,
            &invalid_manifest,
            &invalid_hash,
            &mut BTreeSet::new(),
            0,
            &verify_seed,
        )
        .unwrap_err()
        .contains("does not bind")
    );
}

type PublishOriginal = fn(&Path, &str, &str, &str, &Path, &Path) -> VerifiedReview;

fn assert_experimental_original_roots_unchanged_v1_delta_chain(
    case: &str,
    publish_original: PublishOriginal,
) {
    let repository = crate::test_support::TestRepository::new(case);
    repository.write(".gitignore", ".local-exclude/\n");
    repository.write("owned.txt", "base\n");
    repository.git(["add", ".gitignore", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    let base = repository_head(&repository.path);
    repository.write("owned.txt", "candidate a\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate a"]);
    let candidate_a = repository_head(&repository.path);
    repository.git(["switch", "-c", "slice/direct/review-delta-alpha"]);
    let contract = repository.write(".local-exclude/contract.json", "contract\n");
    let baseline = repository.write(
        ".local-exclude/evidence-a.txt",
        &format!("Candidate: {candidate_a}\n"),
    );
    let seed = publish_original(
        &repository.path,
        &base,
        &candidate_a,
        &candidate_a,
        &contract,
        &baseline,
    );
    let seed_path = resolve_input_path(&repository.path, &seed.manifest_path);
    let verify_seed = |_: &Path, path: &Path, expected: &str| {
        if std::fs::canonicalize(path).unwrap() == std::fs::canonicalize(&seed_path).unwrap()
            && expected == seed.manifest_hash
        {
            Ok(seed.clone())
        } else {
            Err("unexpected alpha seed review".to_owned())
        }
    };

    repository.write("owned.txt", "candidate b\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate b"]);
    let candidate_b = repository_head(&repository.path);
    let inputs = delta_inputs(
        &repository.path,
        seed.clone(),
        &candidate_b,
        "F1",
        format!("Candidate: {candidate_b}\npassed\n").as_bytes(),
        "alpha-b",
    );
    let (manifest, hash, _) = publish_delta_fixture(&repository.path, &inputs);
    let verified = verify_chain_head_with(
        &repository.path,
        &manifest,
        &hash,
        &mut BTreeSet::new(),
        0,
        &verify_seed,
    )
    .unwrap();

    assert_eq!(verified.base_commit, base);
    assert_eq!(verified.candidate_commit, candidate_b);
}

// 이미 발행된 v1alpha1 original manifest가 alpha2 도입 뒤에도 같은 delta-v1 chain
// root로 재현되어 기존 finding-resolution hop을 끝까지 검증한다.
#[test]
fn v1_alpha1_original_roots_unchanged_v1_delta_chain() {
    assert_experimental_original_roots_unchanged_v1_delta_chain(
        "review-delta-alpha1-root",
        crate::review_packet::tests::support::publish_original_v1_alpha1,
    );
}

// sentinel-safe v1alpha2 original manifest도 delta 자체의 frozen v1 bytes/schema를
// 바꾸지 않고 동일한 recursive continuation protocol을 사용한다.
#[test]
fn v1_alpha2_original_roots_unchanged_v1_delta_chain() {
    assert_experimental_original_roots_unchanged_v1_delta_chain(
        "review-delta-alpha2-root",
        crate::review_packet::tests::support::publish_original_v1_alpha2,
    );
}
