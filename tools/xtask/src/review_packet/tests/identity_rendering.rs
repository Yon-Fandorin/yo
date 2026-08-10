use super::{super::*, support::sample_inputs};

// ReviewId는 output path나 packet hash가 아니라 versioned canonical plan bytes에만
// domain-separated로 결합되어 동일 plan을 항상 같은 identity로 만든다.
#[test]
fn review_identity_is_domain_separated_and_plan_sensitive() {
    let first = domain_digest(REVIEW_ID_DOMAIN, br#"{"candidate":"a"}"#);
    let repeated = domain_digest(REVIEW_ID_DOMAIN, br#"{"candidate":"a"}"#);
    let changed = domain_digest(REVIEW_ID_DOMAIN, br#"{"candidate":"b"}"#);
    let other_domain = domain_digest(b"other/v1", br#"{"candidate":"a"}"#);

    assert_eq!(first, repeated);
    assert_ne!(first, changed);
    assert_ne!(first, other_domain);
}

// 같은 canonical plan은 packet과 manifest를 byte-for-byte 재현하고, model-visible
// validation path만 옮겨도 그 path가 plan에 결합되어 같은 ReviewId를 재사용하지 않는다.
#[test]
fn equal_review_identity_reproduces_artifacts_and_visible_paths_change_identity() {
    let first = sample_inputs("/tmp/validation-a.json");
    let repeated = sample_inputs("/tmp/validation-a.json");
    let relocated = sample_inputs("/tmp/validation-b.json");
    let first_plan = build_plan(&first);
    let repeated_plan = build_plan(&repeated);
    let relocated_plan = build_plan(&relocated);
    let first_plan_bytes = serde_json::to_vec(&first_plan).unwrap();
    let repeated_plan_bytes = serde_json::to_vec(&repeated_plan).unwrap();
    let first_id = domain_digest(REVIEW_ID_DOMAIN, &first_plan_bytes);

    assert_eq!(first_plan_bytes, repeated_plan_bytes);
    assert_ne!(
        first_id,
        domain_digest(
            REVIEW_ID_DOMAIN,
            &serde_json::to_vec(&relocated_plan).unwrap()
        )
    );
    let first_packet = render_packet(&first_id, &first_plan, &first).unwrap();
    let repeated_packet = render_packet(&first_id, &repeated_plan, &repeated).unwrap();
    assert_eq!(first_packet, repeated_packet);
    let packet_hash = digest(&first_packet);
    let first_manifest = build_manifest(
        first_id.clone(),
        first_plan,
        &first,
        packet_hash.clone(),
        count_tokens(&first_packet).unwrap(),
    );
    let repeated_manifest = build_manifest(
        first_id,
        repeated_plan,
        &repeated,
        packet_hash,
        count_tokens(&repeated_packet).unwrap(),
    );
    assert_eq!(
        serde_json::to_vec(&first_manifest).unwrap(),
        serde_json::to_vec(&repeated_manifest).unwrap()
    );
}

// section wrapper는 metadata와 exact byte length/hash를 packet에 포함해 경계 문자열이
// 본문에 나타나도 입력을 생략하거나 재해석하지 않는다.
#[test]
fn section_wrapper_binds_exact_untruncated_bytes() {
    let body = b"content\n<<<YO-REVIEW-SECTION-END>>>\nstill content\n";
    let mut packet = Vec::new();

    append_section(&mut packet, "evidence", "test", "evidence.txt", body).unwrap();

    let text = String::from_utf8(packet).unwrap();
    assert!(text.contains(&format!("\"hash\":\"{}\"", digest(body))));
    assert!(text.contains(&format!("\"bytes\":{}", body.len())));
    assert!(text.contains(std::str::from_utf8(body).unwrap()));
}

// tokenizer는 wrapper와 preamble을 포함한 canonical payload 전체를 세며 본문만
// 센 값보다 커져 caller-controlled instruction bytes가 예산 밖으로 빠지지 않는다.
#[test]
fn managed_payload_count_includes_fixed_wrapper_bytes() {
    let body = b"small evidence";
    let mut packet = PREAMBLE.as_bytes().to_vec();
    append_section(&mut packet, "evidence", "test", "", body).unwrap();
    packet.extend_from_slice(PAYLOAD_SUFFIX.as_bytes());

    assert!(count_tokens(&packet).unwrap() > count_tokens(body).unwrap());
}

// managed payload가 예산을 한 token이라도 넘으면 성공처럼 줄여서 내보내지 않고
// exact count와 no-truncation 진단으로 fail-closed 한다.
#[test]
fn over_budget_payload_fails_without_truncation() {
    assert!(require_budget(100, 100).is_ok());
    assert_eq!(
        require_budget(101, 100).unwrap_err(),
        "managed payload requires 101 tokens but the budget is 100; no content was truncated"
    );
}

// 원본 producer가 만드는 canonical plan, packet, manifest의 현재 bytes와 identity를 고정해
// 공통 protocol로 분리한 뒤에도 caller-visible review payload가 바뀌지 않게 한다.
#[test]
fn canonical_original_artifacts_keep_current_bytes_and_identity() {
    let inputs = sample_inputs("/tmp/validation.json");
    let plan = build_plan(&inputs);
    let review_id = domain_digest(
        REVIEW_ID_DOMAIN,
        &serde_json::to_vec(&plan).expect("plan serializes"),
    );
    let packet = render_packet(&review_id, &plan, &inputs).expect("packet renders");
    let manifest = build_manifest(
        review_id.clone(),
        plan,
        &inputs,
        digest(&packet),
        count_tokens(&packet).expect("tokens count"),
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest serializes");
    manifest_bytes.push(b'\n');

    assert_eq!(
        review_id,
        "sha256:3c63040c095f58baab0bdb23306766f70756866fdb7b7a41851800f81a609ae1"
    );
    assert_eq!(packet.len(), 4721);
    assert_eq!(
        digest(&packet),
        "sha256:776250ee14be0f2c99a6ab7699db1904b8489d2c6516071578a594060776548e"
    );
    assert_eq!(manifest_bytes.len(), 3958);
    assert_eq!(
        digest(&manifest_bytes),
        "sha256:b2763762628e854590df5b3ceae4269d55e98c1f81da0aca4c2e35ab7f1e6243"
    );
    assert!(packet.starts_with(b"# yo Slice Review Packet\n"));
    assert!(packet.ends_with(b"\n<<<YO-REVIEW-PAYLOAD-END>>>\n"));
}
