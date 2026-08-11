use super::{
    super::{
        AffectedPathPolicy, MAX_AGGREGATE_EVIDENCE_BYTES,
        capture::captured,
        evidence::{
            TransitionContext, add_evidence_bytes, capture_validation, require_exact_finding_set,
            validate_transition,
        },
        request::validate_request,
    },
    support::{commit, finding, hash, prior, prior_findings},
};
use crate::{
    review_delta::{
        model::{EvidenceRequest, Request, TOKENIZER_PROFILE},
        v1alpha1::{DELIVERY_PROFILE, REQUEST_SCHEMA},
    },
    review_packet,
    review_protocol::{NamedCaptured, digest},
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
    let root = crate::test_support::unique_path("review-delta-transition");
    std::fs::create_dir_all(&root).unwrap();
    let candidate = commit(3);
    let previous_path = root.join("baseline.txt");
    let bound_path = root.join("new-baseline.txt");
    let extra_path = root.join("extra.txt");
    let unbound_path = root.join("unbound.txt");
    std::fs::write(&previous_path, b"old evidence").unwrap();
    std::fs::write(&extra_path, b"extra").unwrap();
    std::fs::write(&unbound_path, b"passed").unwrap();
    let bound_bytes = format!("Candidate: {candidate}\npassed\n").into_bytes();
    std::fs::write(&bound_path, &bound_bytes).unwrap();
    let previous = review_packet::VerifiedEvidence {
        name: "baseline".to_owned(),
        path: previous_path.to_string_lossy().into_owned(),
        hash: digest(b"old evidence"),
    };
    let prior = prior(vec![previous.clone()]);
    let delta = captured("git-delta.patch".to_owned(), b"delta".to_vec()).unwrap();
    let bound = NamedCaptured {
        name: "baseline".to_owned(),
        artifact: captured(bound_path.to_string_lossy().into_owned(), bound_bytes).unwrap(),
    };
    assert!(
        validate_transition(
            TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
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
            TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
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
        artifact: captured(extra_path.to_string_lossy().into_owned(), b"extra".to_vec()).unwrap(),
    };
    assert!(
        validate_transition(
            TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
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
    let same_path_with_new_bytes = NamedCaptured {
        name: previous.name.clone(),
        artifact: captured(
            previous.path.clone(),
            format!("Candidate: {candidate}\nnew result\n").into_bytes(),
        )
        .unwrap(),
    };
    assert!(
        validate_transition(
            TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
            &prior,
            &candidate,
            &delta,
            &[finding("F1")],
            &[],
            &[same_path_with_new_bytes],
        )
        .unwrap_err()
        .contains("new immutable path")
    );
    let unbound = NamedCaptured {
        name: "baseline".to_owned(),
        artifact: captured(
            unbound_path.to_string_lossy().into_owned(),
            b"passed".to_vec(),
        )
        .unwrap(),
    };
    assert!(
        validate_transition(
            TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
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
    std::fs::remove_dir_all(root).unwrap();
}

// 문자열이 다른 `nested/../baseline` alias도 같은 canonical 파일을 가리키면 새 evidence
// path가 아니므로 candidate를 미리 포함한 bytes라도 immutable-path gate에서 거부한다.
#[test]
fn affected_evidence_rejects_a_canonical_alias_of_the_prior_path() {
    let root = crate::test_support::unique_path("review-delta-evidence-alias");
    std::fs::create_dir_all(root.join("nested")).unwrap();
    let candidate = commit(3);
    let bytes = format!("Candidate: {candidate}\nprecomputed result\n").into_bytes();
    let previous_path = root.join("baseline.txt");
    std::fs::write(&previous_path, &bytes).unwrap();
    let prior = prior(vec![review_packet::VerifiedEvidence {
        name: "baseline".to_owned(),
        path: previous_path.to_string_lossy().into_owned(),
        hash: digest(&bytes),
    }]);
    let alias = root.join("nested/../baseline.txt");
    let affected = NamedCaptured {
        name: "baseline".to_owned(),
        artifact: captured(alias.to_string_lossy().into_owned(), bytes).unwrap(),
    };

    let error = validate_transition(
        TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
        &prior,
        &candidate,
        &captured("git-delta.patch".to_owned(), b"delta".to_vec()).unwrap(),
        &[finding("F1")],
        &[],
        &[affected],
    )
    .unwrap_err();

    assert!(error.contains("new immutable path"));
    std::fs::remove_dir_all(root).unwrap();
}

// 초기 검사 때 별도 파일을 가리키던 symlink가 같은 bytes/hash의 prior file로
// retarget되면 captured evidence 비교만으로는 구분되지 않지만, publication 직전과 같은
// transition 재검사가 현재 canonical identity를 다시 읽어 거부함을 확인한다.
#[test]
#[cfg(unix)]
fn canonical_path_gate_detects_alias_retarget_during_revalidation() {
    use std::os::unix::fs::symlink;

    let root = crate::test_support::unique_path("review-delta-evidence-retarget");
    std::fs::create_dir_all(&root).unwrap();
    let candidate = commit(3);
    let bytes = format!("Prior: {}\nCandidate: {candidate}\n", commit(2)).into_bytes();
    let prior_path = root.join("prior.txt");
    let new_path = root.join("new.txt");
    let affected_path = root.join("current.txt");
    std::fs::write(&prior_path, &bytes).unwrap();
    std::fs::write(&new_path, &bytes).unwrap();
    symlink(&new_path, &affected_path).unwrap();
    let prior = prior(vec![review_packet::VerifiedEvidence {
        name: "baseline".to_owned(),
        path: prior_path.to_string_lossy().into_owned(),
        hash: digest(&bytes),
    }]);
    let affected = NamedCaptured {
        name: "baseline".to_owned(),
        artifact: captured(affected_path.to_string_lossy().into_owned(), bytes.clone()).unwrap(),
    };
    let delta = captured("git-delta.patch".to_owned(), b"delta".to_vec()).unwrap();

    validate_transition(
        TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
        &prior,
        &candidate,
        &delta,
        &[finding("F1")],
        &[],
        std::slice::from_ref(&affected),
    )
    .unwrap();
    std::fs::remove_file(&affected_path).unwrap();
    symlink(&prior_path, &affected_path).unwrap();
    let error = validate_transition(
        TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
        &prior,
        &candidate,
        &delta,
        &[finding("F1")],
        &[],
        &[affected],
    )
    .unwrap_err();

    assert!(error.contains("new immutable path"));
    std::fs::remove_dir_all(root).unwrap();
}

// external-operation evidence를 affected로 교체하면 delta validator도 새 candidate를
// 요구하고, 이전 candidate를 담은 구조화 evidence는 일반 문자열 포함 검사 전에 거부한다.
#[test]
fn affected_external_operation_evidence_binds_the_replacement_candidate() {
    let root = crate::test_support::unique_path("review-delta-external-operation");
    std::fs::create_dir_all(&root).unwrap();
    let candidate = commit(3);
    let name = "external-operation/git-amend";
    let previous_path = root.join("old-operation.json");
    let affected_path = root.join("new-operation.json");
    std::fs::write(&previous_path, b"old operation").unwrap();
    std::fs::write(&affected_path, b"new operation").unwrap();
    let prior = prior(vec![review_packet::VerifiedEvidence {
        name: name.to_owned(),
        path: previous_path.to_string_lossy().into_owned(),
        hash: digest(b"old operation"),
    }]);
    let delta = captured("git-delta.patch".to_owned(), b"delta".to_vec()).unwrap();
    let affected = |bound_candidate: &str| NamedCaptured {
        name: name.to_owned(),
        artifact: captured(
            affected_path.to_string_lossy().into_owned(),
            serde_json::to_vec(&serde_json::json!({
                "schema": "yo.external-operation-evidence/v1",
                "candidate_commit": bound_candidate,
                "operation": {
                    "working_directory": ".",
                    "argv": ["git", "commit", "--amend", "--file", "message"],
                    "expected_exit": {"kind": "code", "value": 1},
                    "observed_exit": {"kind": "code", "value": 1}
                },
                "counterfactual": "The amend must fail before changing HEAD.",
                "observations": [{
                    "name": "HEAD",
                    "expected_relation": "equal",
                    "before": "same-head",
                    "after": "same-head"
                }]
            }))
            .unwrap(),
        )
        .unwrap(),
    };

    validate_transition(
        TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
        &prior,
        &candidate,
        &delta,
        &[finding("F1")],
        &[],
        &[affected(&candidate)],
    )
    .unwrap();
    let error = validate_transition(
        TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
        &prior,
        &candidate,
        &delta,
        &[finding("F1")],
        &[],
        &[affected(&commit(2))],
    )
    .unwrap_err();
    assert!(error.contains("does not identify the exact candidate commit"));
    std::fs::remove_dir_all(root).unwrap();
}

// malformed external-operation bytes와 prior와 동일한 path/hash를 함께 주면 frozen v1은
// 기존처럼 구조 오류를 먼저 반환하고, v1alpha1만 새 canonical-path 오류를 먼저 반환해
// 두 wire version의 이중 오류 failure ordering이 서로 섞이지 않음을 확인한다.
#[test]
fn frozen_v1_keeps_structure_before_identity_for_dual_invalid_evidence() {
    let root = crate::test_support::unique_path("review-delta-v1-failure-order");
    std::fs::create_dir_all(&root).unwrap();
    let candidate = commit(3);
    let name = "external-operation/git-amend";
    let evidence_path = root.join("operation.json");
    let malformed = b"not structured operation evidence\n".to_vec();
    std::fs::write(&evidence_path, &malformed).unwrap();
    let prior = prior(vec![review_packet::VerifiedEvidence {
        name: name.to_owned(),
        path: evidence_path.to_string_lossy().into_owned(),
        hash: digest(&malformed),
    }]);
    let affected = NamedCaptured {
        name: name.to_owned(),
        artifact: captured(evidence_path.to_string_lossy().into_owned(), malformed).unwrap(),
    };
    let delta = captured("git-delta.patch".to_owned(), b"delta".to_vec()).unwrap();

    let legacy_error = validate_transition(
        TransitionContext::new(&root, AffectedPathPolicy::LegacyStringIdentity),
        &prior,
        &candidate,
        &delta,
        &[finding("F1")],
        &[],
        std::slice::from_ref(&affected),
    )
    .unwrap_err();
    assert!(legacy_error.contains("invalid external-operation evidence"));
    assert!(!legacy_error.contains("unchanged from the prior candidate"));

    let alpha_error = validate_transition(
        TransitionContext::new(&root, AffectedPathPolicy::CanonicalIdentity),
        &prior,
        &candidate,
        &delta,
        &[finding("F1")],
        &[],
        &[affected],
    )
    .unwrap_err();
    assert!(alpha_error.contains("new immutable path"));
    std::fs::remove_dir_all(root).unwrap();
}
