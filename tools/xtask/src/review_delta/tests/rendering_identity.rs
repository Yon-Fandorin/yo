use super::{
    super::*,
    support::{commit, finding, hash, prior, prior_findings},
};

// continuation은 prior identity와 exact findings를 포함하되 provider-neutral delta에
// 이전 packet 본문 전체를 다시 싣지 않음을 확인한다.
#[test]
fn packet_keeps_prior_identity_without_replaying_the_prior_packet() {
    let prior = prior(Vec::new());
    let prior_findings = prior_findings(&["F1"]);
    let inputs = Inputs {
        request: captured("request.json".to_owned(), b"request".to_vec()).unwrap(),
        prior_manifest: captured("manifest.json".to_owned(), b"prior manifest".to_vec()).unwrap(),
        prior_packet: captured(
            "packet.md".to_owned(),
            b"PRIOR_PACKET_BODY_MUST_NOT_BE_REPLAYED".to_vec(),
        )
        .unwrap(),
        prior_findings,
        prior,
        replacement_candidate: commit(3),
        delta: captured(
            "git-delta.patch".to_owned(),
            b"diff --git a/file b/file\n".to_vec(),
        )
        .unwrap(),
        slice_contract: captured("slice-contract.json".to_owned(), b"slice contract".to_vec())
            .unwrap(),
        findings: vec![finding("F1")],
        reused_validation: Vec::new(),
        affected_validation: vec![NamedCaptured {
            name: "focused".to_owned(),
            artifact: captured("focused.txt".to_owned(), b"focused green\n".to_vec()).unwrap(),
        }],
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: 10_000,
    };
    let plan = build_plan(&inputs);
    let packet = render_packet(&hash(9), &plan, &inputs).unwrap();
    let text = std::str::from_utf8(&packet).unwrap();

    assert!(text.contains(&inputs.prior.review_id));
    assert!(text.contains("F1"));
    assert!(text.contains("focused green"));
    assert!(text.contains("diff --git"));
    assert!(!text.contains("PRIOR_PACKET_BODY_MUST_NOT_BE_REPLAYED"));
}

// 후속 continuation은 매번 원본 candidate부터 diff하지 않고 직전 replacement를
// chain head로 사용함을 확인한다.
#[test]
fn later_continuation_starts_from_the_previous_replacement() {
    let mut chain_head = prior(Vec::new());
    chain_head.review_id = hash(8);
    chain_head.candidate_commit = commit(3);
    chain_head.manifest_path = "first-delta/manifest.json".to_owned();
    chain_head.packet_path = "first-delta/packet.md".to_owned();
    let inputs = Inputs {
        request: captured("request.json".to_owned(), b"request".to_vec()).unwrap(),
        prior_manifest: captured("first-delta/manifest.json".to_owned(), b"manifest".to_vec())
            .unwrap(),
        prior_packet: captured("first-delta/packet.md".to_owned(), b"packet".to_vec()).unwrap(),
        prior_findings: {
            let mut value = prior_findings(&["F2"]);
            let mut parsed: PriorFindings = serde_json::from_slice(&value.bytes).unwrap();
            parsed.review_id = hash(8);
            parsed.candidate_commit = commit(3);
            value.bytes = serde_json::to_vec(&parsed).unwrap();
            value.hash = digest(&value.bytes);
            value
        },
        prior: chain_head,
        replacement_candidate: commit(4),
        delta: captured("git-delta.patch".to_owned(), b"second hop".to_vec()).unwrap(),
        slice_contract: captured("slice-contract.json".to_owned(), b"contract".to_vec()).unwrap(),
        findings: vec![finding("F2")],
        reused_validation: Vec::new(),
        affected_validation: vec![NamedCaptured {
            name: "second-hop".to_owned(),
            artifact: captured("second-hop.txt".to_owned(), b"green".to_vec()).unwrap(),
        }],
        delivery_profile_bytes: delivery_profile_bytes(),
        max_tokens: 10_000,
    };

    let plan = build_plan(&inputs);
    assert_eq!(plan.prior_review_id, hash(8));
    assert_eq!(plan.prior_candidate_commit, commit(3));
    assert_eq!(plan.replacement_candidate_commit, commit(4));
}

// continuation producer가 만드는 canonical delta packet과 manifest의 현재 bytes와 identity를
// 고정해 common chain protocol 분리 뒤에도 같은 reviewer-visible payload를 보존한다.
#[test]
fn canonical_delta_artifacts_keep_current_bytes_and_identity() {
    let inputs = Inputs {
        request: captured("request.json".to_owned(), b"request".to_vec()).unwrap(),
        prior_manifest: captured("manifest.json".to_owned(), b"prior manifest".to_vec()).unwrap(),
        prior_packet: captured("packet.md".to_owned(), b"prior packet".to_vec()).unwrap(),
        prior_findings: prior_findings(&["F1"]),
        prior: prior(Vec::new()),
        replacement_candidate: commit(3),
        delta: captured(
            "git-delta.patch".to_owned(),
            b"diff --git a/file b/file\n".to_vec(),
        )
        .unwrap(),
        slice_contract: captured("slice-contract.json".to_owned(), b"slice contract".to_vec())
            .unwrap(),
        findings: vec![finding("F1")],
        reused_validation: Vec::new(),
        affected_validation: vec![NamedCaptured {
            name: "focused".to_owned(),
            artifact: captured("focused.txt".to_owned(), b"focused green\n".to_vec()).unwrap(),
        }],
        delivery_profile_bytes: super::super::delivery_profile_bytes(),
        max_tokens: 10_000,
    };
    let plan = build_plan(&inputs);
    let review_delta_id = domain_digest(
        REVIEW_DELTA_ID_DOMAIN,
        &serde_json::to_vec(&plan).expect("delta plan serializes"),
    );
    let packet = render_packet(&review_delta_id, &plan, &inputs).expect("delta packet renders");
    let manifest = build_manifest(
        review_delta_id.clone(),
        plan,
        &inputs,
        digest(&packet),
        count_tokens(&packet).expect("tokens count"),
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest serializes");
    manifest_bytes.push(b'\n');

    assert_eq!(
        review_delta_id,
        "sha256:e9225833892b91c75de244fac9c284b0c11b3caa7a5117773f9b3e75b4a332d7"
    );
    assert_eq!(packet.len(), 4708);
    assert_eq!(
        digest(&packet),
        "sha256:da5a8ed3a4b009d86d9212edc3f06e98cf642fee118ee7caa31de980247b8f20"
    );
    assert_eq!(manifest_bytes.len(), 3846);
    assert_eq!(
        digest(&manifest_bytes),
        "sha256:e7fcbe71810dbdb3110a4fbcaf0398adde4dffac35d07aee3fe7c505ffc64a20"
    );
    assert!(packet.starts_with(b"# yo Slice Finding-Resolution Review Delta\n"));
    assert!(packet.ends_with(b"\n<<<YO-REVIEW-DELTA-PAYLOAD-END>>>\n"));
}
