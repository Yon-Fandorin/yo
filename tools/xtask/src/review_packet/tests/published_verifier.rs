use super::{super::*, support::sample_inputs};

fn produced_artifacts(inputs: &Inputs) -> (Manifest, Vec<u8>, Vec<u8>) {
    let plan = build_plan(inputs);
    let review_id = domain_digest(
        REVIEW_ID_DOMAIN,
        &serde_json::to_vec(&plan).expect("plan serializes"),
    );
    let packet = render_packet(&review_id, &plan, inputs).expect("packet renders");
    let manifest = build_manifest(
        review_id,
        plan,
        inputs,
        digest(&packet),
        count_tokens(&packet).expect("tokens count"),
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest serializes");
    manifest_bytes.push(b'\n');
    (manifest, manifest_bytes, packet)
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

    let mut wrong_tokens = manifest.clone();
    wrong_tokens.packet.managed_payload_tokens += 1;
    assert!(
        verify_canonical_artifacts(&wrong_tokens, &manifest_bytes, &packet, &inputs)
            .unwrap_err()
            .contains("token record")
    );
}
