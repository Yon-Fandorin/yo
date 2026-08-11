use super::{
    super::{
        REVIEW_ID_DOMAIN,
        canonical::{build_manifest, build_plan},
        capture::{Inputs, captured},
        model::Manifest,
        render::{count_tokens, render_packet_with_metadata},
        storage,
        verifier::{
            require_base_candidate_provenance, verify_canonical_artifacts, verify_published,
        },
    },
    support::{publish_original, sample_inputs, sample_inputs_v1_alpha1},
};
use crate::review_protocol::{digest, domain_digest};

fn repository_head(repository: &std::path::Path) -> String {
    crate::git::output_in(repository, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned()
}

fn produced_artifacts(inputs: &Inputs) -> (Manifest, Vec<u8>, Vec<u8>) {
    let plan = build_plan(inputs);
    let review_id = domain_digest(
        REVIEW_ID_DOMAIN,
        &serde_json::to_vec(&plan).expect("plan serializes"),
    );
    let rendered = render_packet_with_metadata(&review_id, &plan, inputs).expect("packet renders");
    let manifest = build_manifest(
        review_id,
        plan,
        inputs,
        digest(&rendered.bytes),
        count_tokens(&rendered.bytes).expect("tokens count"),
        rendered.input_prefix,
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest serializes");
    manifest_bytes.push(b'\n');
    (manifest, manifest_bytes, rendered.bytes)
}

// 수기로 만든 shadow manifest가 아니라 실제 review-packet serializer의 산출물을
// continuation verifier에 넣어 producer-consumer 계약이 이어짐을 확인한다.
#[test]
fn canonical_producer_artifacts_are_accepted_by_the_consumer_verifier() {
    let inputs = sample_inputs("/tmp/validation.json");
    let (manifest, manifest_bytes, packet) = produced_artifacts(&inputs);

    verify_canonical_artifacts(&manifest, &manifest_bytes, &packet, &inputs).unwrap();
}

// 나머지 manifest 구조가 그럴듯해도 profile이나 token record가 달라지면
// canonical consumer가 거부함을 확인한다.
#[test]
fn canonical_consumer_rejects_profile_and_token_record_drift() {
    let inputs = sample_inputs("/tmp/validation.json");
    let (manifest, manifest_bytes, packet) = produced_artifacts(&inputs);

    let mut wrong_profile = manifest.clone();
    wrong_profile.plan.tokenizer_profile = "other/v1".to_owned();
    assert!(
        verify_canonical_artifacts(&wrong_profile, &manifest_bytes, &packet, &inputs)
            .unwrap_err()
            .contains("unsupported contract")
    );

    let mut unknown_delivery = manifest.clone();
    unknown_delivery.plan.delivery_profile.id = "yo.slice-review-markdown/unknown".to_owned();
    assert!(
        verify_canonical_artifacts(&unknown_delivery, &manifest_bytes, &packet, &inputs)
            .unwrap_err()
            .contains("unsupported contract")
    );

    let mut wrong_tokens = manifest.clone();
    wrong_tokens.packet.managed_payload_tokens += 1;
    assert!(
        verify_canonical_artifacts(&wrong_tokens, &manifest_bytes, &packet, &inputs)
            .unwrap_err()
            .contains("token record")
    );
}

// v1alpha1 manifest의 prefix 경계·hash·standalone token은 실제 packet 앞부분에서
// 다시 계산되므로 수치만 그럴듯하게 바꾼 manifest를 canonical evidence로 받지 않는다.
#[test]
fn v1_alpha1_consumer_rejects_tampered_prefix_metadata() {
    let inputs = sample_inputs_v1_alpha1("/tmp/validation.json");
    let (manifest, manifest_bytes, packet) = produced_artifacts(&inputs);

    verify_canonical_artifacts(&manifest, &manifest_bytes, &packet, &inputs).unwrap();
    let mut tampered = manifest;
    tampered
        .input_prefix
        .as_mut()
        .expect("v1alpha1 prefix exists")
        .bytes += 1;

    assert!(
        verify_canonical_artifacts(&tampered, &manifest_bytes, &packet, &inputs)
            .unwrap_err()
            .contains("input-prefix record")
    );
}

// 다른 authority로 만든 유효 prefix와 원래 candidate suffix를 이어 붙여도 complete
// canonical packet 재현과 다르므로 partial/reference delivery로 우회할 수 없다.
#[test]
fn v1_alpha1_consumer_rejects_spliced_prefix_and_suffix() {
    let expected = sample_inputs_v1_alpha1("/tmp/validation.json");
    let mut other = sample_inputs_v1_alpha1("/tmp/validation.json");
    other.authorities[0] =
        captured("CONTRIBUTING.md".to_owned(), b"other authority".to_vec()).unwrap();
    let (manifest, manifest_bytes, packet) = produced_artifacts(&expected);
    let (other_manifest, _, other_packet) = produced_artifacts(&other);
    let expected_end = manifest.input_prefix.as_ref().unwrap().bytes;
    let other_end = other_manifest.input_prefix.as_ref().unwrap().bytes;
    let mut spliced = other_packet[..other_end].to_vec();
    spliced.extend_from_slice(&packet[expected_end..]);

    assert!(
        verify_canonical_artifacts(&manifest, &manifest_bytes, &spliced, &expected)
            .unwrap_err()
            .contains("packet does not reproduce")
    );
}

// published verifier는 producer가 반환한 manifest bytes의 hash를 ContextBuild 재생보다 먼저
// 확인해 stale continuation이 다른 입력을 읽기 전에 현재 diagnostic을 보존한다.
#[test]
fn published_verifier_rejects_manifest_hash_drift_before_replay() {
    let repository = crate::test_support::TestRepository::new("review-published-hash");
    let manifest_text = "{\"schema\":\"yo.slice-review-manifest/v1\"}\n";
    let manifest_bytes = manifest_text.as_bytes();
    let manifest_path = repository.write("manifest.json", manifest_text);
    let expected = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    let error = verify_published(&repository.path, &manifest_path, expected).unwrap_err();

    assert_eq!(
        error,
        format!(
            "published Slice review manifest hash mismatch: expected {expected}, found {}",
            digest(manifest_bytes)
        )
    );
}

// manifest의 네 Git revision 중 하나라도 canonical commit 문법이 아니면 non-repository
// 위치에서도 Git root 탐색보다 그 필드 진단이 먼저 나와 untrusted 값을 Git에 넘기지 않는다.
#[test]
fn published_verifier_validates_every_manifest_revision_before_git() {
    let root = crate::test_support::unique_path("review-published-revisions");
    std::fs::create_dir_all(&root).unwrap();
    let inputs = sample_inputs("/tmp/validation.json");
    let (valid, _, _) = produced_artifacts(&inputs);

    for (case, expected_label) in [
        ("base", "published review base"),
        ("candidate", "published review candidate"),
        ("trusted", "published review trusted integration"),
        ("checkpoint", "published review Checkpoint authority basis"),
    ] {
        let mut manifest = valid.clone();
        match case {
            "base" => manifest.plan.base_commit = "--base".to_owned(),
            "candidate" => manifest.plan.candidate_commit = "--candidate".to_owned(),
            "trusted" => manifest.plan.trusted_commit = "--trusted".to_owned(),
            "checkpoint" => {
                manifest.plan.active_checkpoint.authority_basis_commit = "--checkpoint".to_owned();
            },
            _ => unreachable!("closed revision fixture"),
        }
        let mut bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        bytes.push(b'\n');
        let path = root.join(format!("{case}.json"));
        std::fs::write(&path, &bytes).unwrap();

        assert_eq!(
            verify_published(&root, &path, &digest(&bytes)).unwrap_err(),
            format!("{expected_label} must be a full lowercase SHA-1 commit ID")
        );
    }

    std::fs::remove_dir_all(root).unwrap();
}

// 40자리 annotated-tag object ID는 commit으로 peel될 수 있어도 manifest가 그 exact
// commit ID를 직접 이름 붙인 것이 아니므로 ContextBuild나 authority 재생 전에 거부한다.
#[test]
fn published_verifier_rejects_a_tag_object_as_the_candidate_commit() {
    let repository = crate::test_support::TestRepository::new("review-published-tag-object");
    repository.write(".gitignore", ".local-exclude/\n");
    repository.write("owned.txt", "base\n");
    repository.git(["add", ".gitignore", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    let base = repository_head(&repository.path);
    repository.write("owned.txt", "candidate\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate"]);
    let candidate = repository_head(&repository.path);
    repository.git([
        "tag",
        "--annotate",
        "candidate-tag",
        "--message",
        "candidate tag",
        &candidate,
    ]);
    let tag_object = crate::git::output_in(
        &repository.path,
        &["rev-parse", "refs/tags/candidate-tag"],
        false,
    )
    .unwrap()
    .trim()
    .to_owned();
    let contract = repository.write(".local-exclude/contract.json", "contract\n");
    let validation = repository.write(".local-exclude/validation.txt", "passed\n");
    let published = publish_original(
        &repository.path,
        &base,
        &tag_object,
        &candidate,
        &contract,
        &validation,
    );

    let error = verify_published(
        &repository.path,
        &repository.path.join(&published.manifest_path),
        &published.manifest_hash,
    )
    .unwrap_err();

    assert_eq!(
        error,
        "published review candidate revision does not name the exact commit object"
    );
}

// 실제 descendant commit 두 개는 exact object와 ancestry 검사를 모두 통과해 새 gate가
// 정상 published history까지 거부하지 않음을 같은 trusted Git 경로로 확인한다.
#[test]
fn published_provenance_accepts_exact_descendant_commit_objects() {
    let repository = crate::test_support::TestRepository::new("review-published-descendant");
    repository.write("owned.txt", "base\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    let base = repository_head(&repository.path);
    repository.write("owned.txt", "candidate\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "candidate"]);
    let candidate = repository_head(&repository.path);

    require_base_candidate_provenance(&repository.path, &base, &candidate).unwrap();
}

// canonical serializer가 만든 artifact라도 candidate object가 저장소에 없으면 diff나
// model-visible 입력을 읽기 전에 exact commit resolution 단계에서 실패한다.
#[test]
fn published_verifier_rejects_a_missing_candidate_commit_before_input_replay() {
    let repository = crate::test_support::TestRepository::new("review-published-missing-object");
    repository.write("owned.txt", "base\n");
    repository.git(["add", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    let base = repository_head(&repository.path);
    let mut inputs = sample_inputs("/tmp/validation.json");
    inputs.base_commit = base.clone();
    inputs.candidate_commit = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_owned();
    inputs.context.result.trusted_commit = base;
    let (manifest, manifest_bytes, packet) = produced_artifacts(&inputs);
    let directory = repository
        .path
        .join(".local-exclude/methexis/slice-reviews")
        .join(manifest.review_id.strip_prefix("sha256:").unwrap());
    storage::publish(&directory, &packet, &manifest_bytes, || Ok(())).unwrap();
    let manifest_path = directory.join("manifest.json");

    let error =
        verify_published(&repository.path, &manifest_path, &digest(&manifest_bytes)).unwrap_err();

    assert!(error.starts_with("cannot resolve published review candidate revision:"));
}

// base와 candidate가 각각 존재하는 commit이어도 서로 무관한 history면 canonical diff를
// 만들 수 있다는 이유만으로 accepted provenance가 되지 않고 ancestry gate에서 거부된다.
#[test]
fn published_verifier_rejects_unrelated_base_and_candidate_histories() {
    let repository = crate::test_support::TestRepository::new("review-published-unrelated");
    repository.write(".gitignore", ".local-exclude/\n");
    repository.write("owned.txt", "base\n");
    repository.git(["add", ".gitignore", "owned.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    let base = repository_head(&repository.path);
    repository.git(["switch", "--orphan", "unrelated"]);
    repository.write("owned.txt", "unrelated candidate\n");
    repository.git(["add", "--all"]);
    repository.git(["commit", "--quiet", "-m", "unrelated candidate"]);
    let candidate = repository_head(&repository.path);
    repository.git(["switch", "develop"]);
    let contract = repository.write(".local-exclude/contract.json", "contract\n");
    let validation = repository.write(".local-exclude/validation.txt", "passed\n");
    let published = publish_original(
        &repository.path,
        &base,
        &candidate,
        &base,
        &contract,
        &validation,
    );

    let error = verify_published(
        &repository.path,
        &repository.path.join(&published.manifest_path),
        &published.manifest_hash,
    )
    .unwrap_err();

    assert_eq!(
        error,
        "published review base is not an ancestor of its candidate"
    );
}
