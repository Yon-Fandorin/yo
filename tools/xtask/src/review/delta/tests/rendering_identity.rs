use super::{
    super::{
        Inputs, WireContract,
        capture::captured,
        render::{
            build_manifest_for, build_plan_for, count_tokens, delivery_profile_bytes_for,
            render_packet,
        },
        v1, v1alpha1,
    },
    support::{commit, finding, hash, prior, prior_findings},
};
use crate::{
    review::delta::model::PriorFindings,
    review_protocol::{NamedCaptured, digest, domain_digest},
};

// continuation은 prior identity와 exact findings를 포함하되 provider-neutral delta에
// 이전 packet 본문 전체를 다시 싣지 않음을 확인한다.
#[test]
fn packet_keeps_prior_identity_without_replaying_the_prior_packet() {
    let contract = v1alpha1::contract();
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
        delivery_profile_bytes: delivery_profile_bytes_for(contract),
        max_tokens: 10_000,
    };
    let plan = build_plan_for(&inputs, contract);
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
    let contract = v1alpha1::contract();
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
        delivery_profile_bytes: delivery_profile_bytes_for(contract),
        max_tokens: 10_000,
    };

    let plan = build_plan_for(&inputs, contract);
    assert_eq!(plan.prior_review_id, hash(8));
    assert_eq!(plan.prior_candidate_commit, commit(3));
    assert_eq!(plan.replacement_candidate_commit, commit(4));
}

fn identity_inputs(contract: WireContract) -> Inputs {
    Inputs {
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
        delivery_profile_bytes: delivery_profile_bytes_for(contract),
        max_tokens: 10_000,
    }
}

fn identity_artifacts(contract: WireContract) -> (String, Vec<u8>, Vec<u8>) {
    let inputs = identity_inputs(contract);
    let plan = build_plan_for(&inputs, contract);
    let review_delta_id = domain_digest(
        contract.review_id_domain,
        &serde_json::to_vec(&plan).expect("delta plan serializes"),
    );
    let packet = render_packet(&review_delta_id, &plan, &inputs).expect("delta packet renders");
    let manifest = build_manifest_for(
        review_delta_id.clone(),
        plan,
        &inputs,
        digest(&packet),
        count_tokens(&packet).expect("tokens count"),
        contract,
    );
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest serializes");
    manifest_bytes.push(b'\n');
    (review_delta_id, packet, manifest_bytes)
}

// frozen delta-v1의 canonical packet, manifest bytes, identity를 고정해 v1alpha1
// producer를 추가해도 이미 발행된 continuation을 같은 값으로 재현한다.
#[test]
fn legacy_v1_delta_artifacts_keep_frozen_bytes_and_identity() {
    let (review_delta_id, packet, manifest_bytes) = identity_artifacts(v1::contract());

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

// 새 v1alpha1 producer는 frozen v1과 다른 schema/profile/domain identity를 사용하되
// 동일한 공통 renderer로 완전한 packet과 canonical manifest를 결정적으로 만든다.
#[test]
fn v1_alpha1_delta_artifacts_have_a_distinct_canonical_identity() {
    let (legacy_id, _, _) = identity_artifacts(v1::contract());
    let (review_delta_id, packet, manifest_bytes) = identity_artifacts(v1alpha1::contract());
    let manifest: crate::review::delta::model::Manifest =
        serde_json::from_slice(&manifest_bytes).unwrap();

    assert_eq!(
        review_delta_id,
        "sha256:a7514a1fd30e17991d293aaa96ef4bf0db0afb8afdec6f80ddc157afc2706724"
    );
    assert_ne!(review_delta_id, legacy_id);
    assert_eq!(packet.len(), 4720);
    assert_eq!(
        digest(&packet),
        "sha256:d8672b6129d573c22039e7e67fb252e06f32890eb0bd7c034c82b8214db7ce08"
    );
    assert_eq!(manifest_bytes.len(), 3864);
    assert_eq!(
        digest(&manifest_bytes),
        "sha256:59801674eaf20b797e2ffd47900b8d82aeaf9bea1df994c2164e7cd16004db00"
    );
    assert_eq!(manifest.schema, v1alpha1::MANIFEST_SCHEMA);
    assert_eq!(manifest.plan.schema, v1alpha1::PLAN_SCHEMA);
    assert_eq!(
        manifest.plan.delivery_profile.id,
        v1alpha1::DELIVERY_PROFILE
    );
    assert_eq!(manifest.review_delta_id, review_delta_id);
    assert_eq!(manifest.packet.hash, digest(&packet));
}
