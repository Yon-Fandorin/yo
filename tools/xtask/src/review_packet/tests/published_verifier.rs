use super::{
    super::{
        REVIEW_ID_DOMAIN,
        canonical::{build_manifest, build_plan},
        capture::{Inputs, captured},
        model::Manifest,
        render::{count_tokens, render_packet_with_metadata},
        verifier::{verify_canonical_artifacts, verify_published},
    },
    support::{sample_inputs, sample_inputs_v1_alpha1},
};
use crate::review_protocol::{digest, domain_digest};

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
