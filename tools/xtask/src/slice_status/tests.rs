use std::{collections::BTreeSet, fs, path::PathBuf};

use super::{
    action::{next_action, next_invocation},
    delivery,
    lineage::{branch_names_slice, scan_review_lineage, validate_slice_name},
    model::{
        Artifacts, CoordinationScope, EffectiveValidation, ReviewLineage, ScanBudget, SliceState,
    },
    scan::{collect_json, scan_coordination},
};
use crate::{
    git, review_protocol, slice_contract,
    test_support::{TestRepository, unique_path},
};

fn state(head: &str) -> SliceState {
    SliceState {
        worktree: PathBuf::from("/tmp/example"),
        branch: "refs/heads/slice/direct/example".to_owned(),
        head: head.to_owned(),
        bound: slice_contract::BoundSlice {
            slice: "example".to_owned(),
            base: "base".to_owned(),
            base_ref: "refs/heads/develop".to_owned(),
            binding_path: PathBuf::from("binding"),
            contract_path: PathBuf::from("contract"),
            contract_id: "sha256:contract".to_owned(),
        },
        clean: true,
    }
}

// 현재 후보 packet이 이미 있으면 같은 packet을 다시 만들라고 하지 않고 delivery로,
// 이전 후보만 있으면 전체 packet 대신 review delta로 다음 행동을 구분합니다.
#[test]
fn next_action_reuses_current_review_or_selects_delta() {
    let current = ReviewLineage {
        packets: 1,
        latest_candidate: Some("current".to_owned()),
        status: "preserved",
        current_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
        latest_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
        current_validations: Vec::new(),
    };
    assert_eq!(
        next_action(&state("current"), &current, &Artifacts::default()),
        "deliver_current_review"
    );

    let prior = ReviewLineage {
        packets: 1,
        latest_candidate: Some("prior".to_owned()),
        status: "preserved",
        current_review_ids: BTreeSet::new(),
        latest_review_ids: BTreeSet::from(["sha256:prior".to_owned()]),
        current_validations: Vec::new(),
    };
    assert_eq!(
        next_action(
            &state("current"),
            &prior,
            &Artifacts {
                prior_findings: 1,
                ..Artifacts::default()
            },
        ),
        "review_delta"
    );
    assert_eq!(
        next_action(&state("current"), &prior, &Artifacts::default()),
        "build_review"
    );
}

// receipt와 current-candidate gate가 있으면 review/gate 생성을 반복하지 않고 기존
// exact gate 실행으로 바로 이어져 content-addressed reuse 경계를 보존합니다.
#[test]
fn next_action_reuses_existing_gate_inputs() {
    let reviews = ReviewLineage {
        packets: 1,
        latest_candidate: Some("current".to_owned()),
        status: "preserved",
        current_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
        latest_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
        current_validations: Vec::new(),
    };
    let artifacts = Artifacts {
        review_rounds: 1,
        gate_requests: 1,
        ..Artifacts::default()
    };

    assert_eq!(
        next_action(&state("current"), &reviews, &artifacts),
        "run_gate"
    );
}

// 패킷 없는 직접 검토의 gate를 재사용하되 과거 후보·미지원 스키마·저장된 결과는
// 실행 요청으로 취급하지 않습니다. 복수 요청은 임의 선택 대신 모호성을 알립니다.
#[test]
fn direct_review_gate_discovery_returns_exact_request_without_granting_approval() {
    let root = unique_path("slice-status-direct-gate");
    fs::create_dir_all(&root).unwrap();
    let reviews = ReviewLineage {
        packets: 0,
        latest_candidate: None,
        status: "preserved",
        current_review_ids: BTreeSet::new(),
        latest_review_ids: BTreeSet::new(),
        current_validations: Vec::new(),
    };
    let scan = || {
        scan_coordination(
            &root,
            &CoordinationScope {
                repository: &root,
                workspace: &root,
                candidate: "current",
                current_review_ids: &reviews.current_review_ids,
                latest_review_ids: &reviews.latest_review_ids,
                current_validations: &[],
            },
            &mut ScanBudget::default(),
        )
        .unwrap()
    };
    for (name, schema, candidate) in [
        ("stale", "yo.slice-gate-request/v1alpha1", "prior"),
        ("future", "yo.slice-gate-request/v999", "current"),
        ("result", "yo.slice-gate-result/v1alpha1", "current"),
    ] {
        fs::write(
            root.join(format!("{name}.json")),
            serde_json::to_vec(&serde_json::json!({
                "schema": schema,
                "candidate_commit": candidate,
                "next_action": "integrate"
            }))
            .unwrap(),
        )
        .unwrap();
    }
    assert_eq!(scan().gate_requests, 0);
    assert_eq!(
        next_action(&state("current"), &reviews, &scan()),
        "build_review"
    );

    // 발견은 검증이 아닙니다. 불완전한 입력도 gate 실행만 제안하며 승인으로 해석하지 않습니다.
    let request = root.join("gate request.json");
    fs::write(
        &request,
        br#"{"schema":"yo.slice-gate-request/v1alpha1","candidate_commit":"current"}"#,
    )
    .unwrap();
    let artifacts = scan();
    let action = next_action(&state("current"), &reviews, &artifacts);
    assert_eq!(action, "run_gate");
    let (argv, reason) = next_invocation("example", action, &artifacts);
    assert_eq!(
        argv.unwrap(),
        ["cargo", "xtask", "slice", "gate", request.to_str().unwrap()]
    );
    assert!(reason.is_none());
    let mut dirty = state("current");
    dirty.clean = false;
    assert_eq!(next_action(&dirty, &reviews, &artifacts), "clean_candidate");
    let mut broken = reviews;
    broken.status = "broken";
    assert_eq!(
        next_action(&state("current"), &broken, &artifacts),
        "restore_review_lineage"
    );

    fs::copy(&request, root.join("other-request.json")).unwrap();
    let artifacts = scan_coordination(
        &root,
        &CoordinationScope {
            repository: &root,
            workspace: &root,
            candidate: "current",
            current_review_ids: &BTreeSet::new(),
            latest_review_ids: &BTreeSet::new(),
            current_validations: &[],
        },
        &mut ScanBudget::default(),
    )
    .unwrap();
    let (argv, reason) = next_invocation("example", "run_gate", &artifacts);
    assert!(argv.is_none());
    assert!(reason.unwrap().contains("found 2"));
    fs::remove_dir_all(root).unwrap();
}

// Slice 이름과 branch matcher는 임의 경로나 다른 Wave의 접두사를 느슨하게
// 받아들이지 않고 direct 또는 정확한 한 Wave branch만 선택합니다.
#[test]
fn branch_matching_is_exact() {
    assert!(branch_names_slice(
        "refs/heads/slice/direct/example",
        "example"
    ));
    assert!(branch_names_slice(
        "refs/heads/slice/wave-a/example",
        "example"
    ));
    assert!(!branch_names_slice(
        "refs/heads/slice/wave-a/example/extra",
        "example"
    ));
    assert!(validate_slice_name("../example").is_err());
}

// 이미 발행한 후보를 reset으로 버리고 다른 child를 만들면 manifest 자체가 남아
// 있어도 ancestry guard가 rewritten history를 preserved로 오인하지 않습니다.
#[test]
fn review_lineage_detects_replaced_candidate() {
    let repository = TestRepository::new("slice-status-lineage");
    repository.write("tracked.txt", "base\n");
    repository.git(["add", "tracked.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    let base = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    repository.write("tracked.txt", "reviewed\n");
    repository.git(["add", "tracked.txt"]);
    repository.git(["commit", "--quiet", "-m", "reviewed"]);
    let reviewed = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    repository.git(["reset", "--hard", &base]);
    repository.write("tracked.txt", "replacement\n");
    repository.git(["add", "tracked.txt"]);
    repository.git(["commit", "--quiet", "-m", "replacement"]);
    let replacement = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    repository.write(
        ".local-exclude/methexis/review/manifest.json",
        &format!(
            "{{\"plan\":{{\"candidate_commit\":\"{reviewed}\",\"slice_contract\":{{\"hash\":\"sha256:contract\"}}}}}}\n"
        ),
    );
    let state = SliceState {
        worktree: repository.path.clone(),
        branch: "refs/heads/slice/direct/example".to_owned(),
        head: replacement,
        bound: slice_contract::BoundSlice {
            slice: "example".to_owned(),
            base,
            base_ref: "refs/heads/develop".to_owned(),
            binding_path: repository.path.join("binding"),
            contract_path: repository.path.join("contract"),
            contract_id: "sha256:contract".to_owned(),
        },
        clean: true,
    };

    let reviews =
        scan_review_lineage(&state, &repository.path, &mut ScanBudget::default()).unwrap();
    assert_eq!(reviews.packets, 1);
    assert_eq!(reviews.latest_candidate, None);
    assert_eq!(reviews.status, "broken");
}

// 이전 후보의 validation/gate/claim 파일이 coordination에 남아 있어도 현재 HEAD의
// 완료 수로 합산하지 않아 compact status가 stale progress를 만들지 않습니다.
#[test]
fn coordination_counts_only_candidate_bound_progress() {
    let root = unique_path("slice-status-coordination");
    fs::create_dir_all(&root).unwrap();
    for (name, value) in [
        (
            "current-validation.json",
            serde_json::json!({
                "schema": "yo.validation-run-summary/v1alpha2",
                "name": "current-validation",
                "status": "passed",
                "log_hash": "sha256:log",
                "head_commit": "current"
            }),
        ),
        (
            "stale-validation.json",
            serde_json::json!({
                "schema": "yo.validation-run-summary/v1alpha2",
                "head_commit": "stale"
            }),
        ),
        (
            "stale-gate.json",
            serde_json::json!({
                "schema": "yo.slice-gate-request/v1alpha1",
                "candidate_commit": "stale"
            }),
        ),
        (
            "current-receipt.json",
            serde_json::json!({
                "schema": "yo.external-review-delivery-receipt/v1",
                "review_id": "sha256:current-review"
            }),
        ),
        (
            "prior-findings.json",
            serde_json::json!({
                "schema": "yo.slice-review-findings/v1",
                "review_id": "sha256:current-review",
                "candidate_commit": "reviewed"
            }),
        ),
        (
            "stale-receipt.json",
            serde_json::json!({
                "schema": "yo.external-review-delivery-receipt/v1",
                "review_id": "sha256:stale-review"
            }),
        ),
    ] {
        fs::write(root.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }

    let current_validation_path = root.join("current-validation.json");
    let current_validation_hash =
        review_protocol::digest(&fs::read(&current_validation_path).unwrap());
    let effective = [EffectiveValidation {
        name: "current-validation".to_owned(),
        path: current_validation_path,
        hash: current_validation_hash,
        reused: true,
    }];
    let artifacts = scan_coordination(
        &root,
        &CoordinationScope {
            repository: &root,
            workspace: &root,
            candidate: "current",
            current_review_ids: &BTreeSet::from(["sha256:current-review".to_owned()]),
            latest_review_ids: &BTreeSet::from(["sha256:current-review".to_owned()]),
            current_validations: &effective,
        },
        &mut ScanBudget::default(),
    )
    .unwrap();
    assert_eq!(artifacts.validations.len(), 1);
    assert!(artifacts.validations[0].reused);
    assert_eq!(artifacts.gate_requests, 0);
    assert_eq!(artifacts.delivery_receipts, 1);
    assert_eq!(artifacts.review_rounds, 1);
    assert_eq!(artifacts.prior_findings, 1);
    fs::remove_dir_all(root).unwrap();
}

// claim은 외부 효과가 아직 관측되지 않았더라도 exact-once 소유권을 소비하므로 compact
// coordinator는 같은 review에 대한 두 번째 delivery 명령을 절대 제안하지 않습니다.
#[test]
fn current_claim_blocks_a_second_delivery() {
    let root = unique_path("slice-status-current-claim");
    fs::create_dir_all(root.join("attempt")).unwrap();
    fs::write(
        root.join("attempt/claim.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "yo.external-review-delivery-claim/v1alpha2",
            "request_id": "sha256:request",
            "review_id": "sha256:review",
            "candidate_commit": "current"
        }))
        .unwrap(),
    )
    .unwrap();

    let artifacts = scan_coordination(
        &root,
        &CoordinationScope {
            repository: &root,
            workspace: &root,
            candidate: "current",
            current_review_ids: &BTreeSet::from(["sha256:review".to_owned()]),
            latest_review_ids: &BTreeSet::from(["sha256:review".to_owned()]),
            current_validations: &[],
        },
        &mut ScanBudget::default(),
    )
    .unwrap();
    let reviews = ReviewLineage {
        packets: 1,
        latest_candidate: Some("current".to_owned()),
        status: "preserved",
        current_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
        latest_review_ids: BTreeSet::from(["sha256:review".to_owned()]),
        current_validations: Vec::new(),
    };
    assert_eq!(artifacts.delivery.state, delivery::State::Claimed);
    assert_eq!(
        next_action(&state("current"), &reviews, &artifacts),
        "await_current_delivery"
    );
    fs::remove_dir_all(root).unwrap();
}

// coordination에 이전 후보의 유일한 delivery request만 남아 있어도 그 request가
// 가리키는 manifest ReviewId를 다시 결속해 현재 review의 exact argv로 제안하지 않습니다.
#[test]
fn stale_delivery_request_is_not_current_next_argv() {
    let repository = unique_path("slice-status-stale-delivery");
    let coordination = repository.join("coordination");
    let manifest_path = repository.join("stale-manifest.json");
    fs::create_dir_all(&coordination).unwrap();
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schema": "yo.slice-review-manifest/v1",
        "review_id": "sha256:stale-review"
    }))
    .unwrap();
    fs::write(&manifest_path, &manifest).unwrap();
    let egress = serde_json::to_vec(&serde_json::json!({
        "schema": "yo.slice-review-delegated-egress-request/v1alpha1",
        "manifest_path": manifest_path.display().to_string(),
        "manifest_hash": review_protocol::digest(&manifest)
    }))
    .unwrap();
    let egress_path = coordination.join("stale-egress.json");
    fs::write(&egress_path, &egress).unwrap();
    fs::write(
        coordination.join("review-delivery.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema": "yo.slice-review-delegated-delivery-request/v1alpha2",
            "egress_request_path": egress_path.display().to_string(),
            "egress_request_hash": review_protocol::digest(&egress)
        }))
        .unwrap(),
    )
    .unwrap();

    let artifacts = scan_coordination(
        &coordination,
        &CoordinationScope {
            repository: &repository,
            workspace: &repository,
            candidate: "current",
            current_review_ids: &BTreeSet::from(["sha256:current-review".to_owned()]),
            latest_review_ids: &BTreeSet::from(["sha256:current-review".to_owned()]),
            current_validations: &[],
        },
        &mut ScanBudget::default(),
    )
    .unwrap();
    assert!(artifacts.delivery_request.is_none());
    assert_eq!(artifacts.superseded, 1);
    fs::remove_dir_all(repository).unwrap();
}

// original manifest의 review_id와 finding-resolution manifest의 review_delta_id는
// 서로 다른 wire 필드이므로 현재 replacement 후보의 receipt 연결은 둘 다 읽습니다.
#[test]
fn review_lineage_identifies_current_delta_review() {
    let repository = TestRepository::new("slice-status-delta-review");
    repository.write("tracked.txt", "base\n");
    repository.git(["add", "tracked.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    let base = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    repository.write("tracked.txt", "replacement\n");
    repository.git(["add", "tracked.txt"]);
    repository.git(["commit", "--quiet", "-m", "replacement"]);
    let replacement = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned();
    repository.write(
        ".local-exclude/methexis/delta/manifest.json",
        &format!(
            "{{\"review_delta_id\":\"sha256:delta\",\"plan\":{{\"replacement_candidate_commit\":\"{replacement}\",\"slice_contract\":{{\"hash\":\"sha256:contract\"}}}}}}\n"
        ),
    );
    let state = SliceState {
        worktree: repository.path.clone(),
        branch: "refs/heads/slice/direct/example".to_owned(),
        head: replacement,
        bound: slice_contract::BoundSlice {
            slice: "example".to_owned(),
            base,
            base_ref: "refs/heads/develop".to_owned(),
            binding_path: repository.path.join("binding"),
            contract_path: repository.path.join("contract"),
            contract_id: "sha256:contract".to_owned(),
        },
        clean: true,
    };

    let reviews =
        scan_review_lineage(&state, &repository.path, &mut ScanBudget::default()).unwrap();
    assert_eq!(
        reviews.current_review_ids,
        BTreeSet::from(["sha256:delta".to_owned()])
    );
}

// manifest와 coordination은 하나의 전역 예산을 공유해 두 scan이 각각 256개를
// 읽는 방식으로 문서화된 bounded 입력 한도를 우회하지 않습니다.
#[test]
fn json_scan_budget_is_global_across_roots() {
    let first = unique_path("slice-status-budget-first");
    let second = unique_path("slice-status-budget-second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    for index in 0..128 {
        fs::write(first.join(format!("{index:03}.json")), b"{}\n").unwrap();
        fs::write(second.join(format!("{index:03}.json")), b"{}\n").unwrap();
    }
    let mut budget = ScanBudget::default();
    let mut files = Vec::new();
    collect_json(&first, 0, &mut files, &mut budget).unwrap();
    collect_json(&second, 0, &mut files, &mut budget).unwrap();
    fs::write(second.join("overflow.json"), b"{}\n").unwrap();
    assert!(
        collect_json(&second, 0, &mut Vec::new(), &mut budget)
            .unwrap_err()
            .contains("global 256-JSON-file")
    );
    fs::remove_dir_all(first).unwrap();
    fs::remove_dir_all(second).unwrap();
}
