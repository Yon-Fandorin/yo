use super::{
    super::*,
    support::{commit, finding, hash, prior, prior_findings},
};

// continuation에 finding, 양수 budget, replacement 전용 evidence가 모두 있어야
// 변경 후보를 검토하지 않은 빈 성공을 만들 수 없음을 확인한다.
#[test]
fn request_requires_findings_budget_and_affected_evidence() {
    let mut request = Request {
        schema: REQUEST_SCHEMA.to_owned(),
        prior_manifest_path: "manifest.json".to_owned(),
        prior_manifest_hash: hash(1),
        prior_findings_path: "findings.json".to_owned(),
        prior_findings_hash: hash(2),
        finding_dispositions: Vec::new(),
        reused_validation_evidence: Vec::new(),
        affected_validation_evidence: Vec::new(),
        delivery_profile: DELIVERY_PROFILE.to_owned(),
        tokenizer_profile: TOKENIZER_PROFILE.to_owned(),
        max_managed_payload_tokens: 100,
    };

    assert_eq!(
        validate_request(&request).unwrap_err(),
        "at least one finding disposition is required"
    );
    request.finding_dispositions.push(finding("F1"));
    assert!(
        validate_request(&request)
            .unwrap_err()
            .contains("affected validation evidence")
    );
    request.affected_validation_evidence.push(EvidenceRequest {
        name: "focused".to_owned(),
        path: "focused.txt".to_owned(),
    });
    request.max_managed_payload_tokens = 0;
    assert_eq!(
        validate_request(&request).unwrap_err(),
        "managed payload token budget must be positive"
    );
}

// reviewer가 작성한 findings artifact의 모든 ID와 오직 그 ID만 disposition에
// 대응해야 적격 continuation이 됨을 확인한다.
#[test]
fn dispositions_must_match_the_exact_prior_finding_set() {
    let exact = prior_findings(&["F1", "F2"]);
    assert!(require_exact_finding_set(&exact, &[finding("F2"), finding("F1")]).is_ok());
    assert!(
        require_exact_finding_set(&exact, &[finding("F1")])
            .unwrap_err()
            .contains("exact prior finding ID set")
    );
    assert!(
        require_exact_finding_set(&exact, &[finding("F1"), finding("F3")])
            .unwrap_err()
            .contains("exact prior finding ID set")
    );
}

// 이전 evidence 이름을 빠짐없이 reused 또는 affected로 분류하고 두 분류가
// 겹치지 않아야 함을 확인한다.
#[test]
fn validation_capture_requires_complete_non_overlapping_classification() {
    let root = crate::test_support::unique_path("review-delta-evidence");
    std::fs::create_dir_all(&root).unwrap();
    let stable_path = root.join("stable.txt");
    let changed_path = root.join("changed.txt");
    std::fs::write(&stable_path, b"stable green\n").unwrap();
    std::fs::write(&changed_path, b"new focused green\n").unwrap();
    let prior = prior(vec![review_packet::VerifiedEvidence {
        name: "stable".to_owned(),
        path: stable_path.to_string_lossy().into_owned(),
        hash: digest(b"stable green\n"),
    }]);

    assert!(
        capture_validation(&root, &prior, &[], &[])
            .unwrap_err()
            .contains("must be classified")
    );
    assert!(
        capture_validation(
            &root,
            &prior,
            &["stable".to_owned()],
            &[EvidenceRequest {
                name: "stable".to_owned(),
                path: changed_path.to_string_lossy().into_owned(),
            }],
        )
        .unwrap_err()
        .contains("both reused and affected")
    );
    let (reused, affected) =
        capture_validation(&root, &prior, &["stable".to_owned()], &[]).unwrap();
    assert_eq!(reused.len(), 1);
    assert!(affected.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

// 다음 evidence를 보관하는 순간 누적 제한을 넘으면 즉시 거부함을 확인한다.
#[test]
fn aggregate_evidence_limit_is_checked_incrementally() {
    let mut total = MAX_AGGREGATE_EVIDENCE_BYTES - 1;
    assert!(add_evidence_bytes(&mut total, 1).is_ok());
    assert!(
        add_evidence_bytes(&mut total, 1)
            .unwrap_err()
            .contains("aggregate validation evidence")
    );
}

// 생성과 재생이 같은 validator를 사용해, 자체 일관된 bytes만으로 producer의
// 적격성 규칙을 우회할 수 없음을 확인한다.
#[test]
fn transition_validator_rejects_noncanonical_or_unrelated_evidence() {
    let candidate = commit(3);
    let previous = review_packet::VerifiedEvidence {
        name: "baseline".to_owned(),
        path: "baseline.txt".to_owned(),
        hash: digest(b"old evidence"),
    };
    let prior = prior(vec![previous.clone()]);
    let delta = captured("git-delta.patch".to_owned(), b"delta".to_vec()).unwrap();
    let bound = NamedCaptured {
        name: "baseline".to_owned(),
        artifact: captured(
            "new-baseline.txt".to_owned(),
            format!("Candidate: {candidate}\npassed\n").into_bytes(),
        )
        .unwrap(),
    };
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[finding("F1")],
            &[],
            std::slice::from_ref(&bound),
        )
        .is_ok()
    );

    let mut blank = finding("F1");
    blank.summary.clear();
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[blank],
            &[],
            std::slice::from_ref(&bound),
        )
        .is_err()
    );
    let extra_reused = NamedCaptured {
        name: "unrelated".to_owned(),
        artifact: captured("extra.txt".to_owned(), b"extra".to_vec()).unwrap(),
    };
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[finding("F1")],
            &[extra_reused],
            std::slice::from_ref(&bound),
        )
        .unwrap_err()
        .contains("unknown reused")
    );
    let unchanged = NamedCaptured {
        name: previous.name.clone(),
        artifact: captured(previous.path.clone(), b"old evidence".to_vec()).unwrap(),
    };
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[finding("F1")],
            &[],
            &[unchanged],
        )
        .unwrap_err()
        .contains("unchanged")
    );
    let unbound = NamedCaptured {
        name: "baseline".to_owned(),
        artifact: captured("new.txt".to_owned(), b"passed".to_vec()).unwrap(),
    };
    assert!(
        validate_transition(
            &prior,
            &candidate,
            &delta,
            &[finding("F1")],
            &[],
            &[unbound],
        )
        .unwrap_err()
        .contains("does not bind")
    );
}
