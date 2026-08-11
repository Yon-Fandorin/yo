use super::{
    super::{model::Effect, observation, prepare, prepare_with_post_binding, run},
    support::{Fixture, output},
};

// contract publication 뒤 branch만 남은 중단 상태는 exact base와 request가
// 일치할 때 새 worktree와 binding만 복구하여 수동 ref 삭제를 요구하지 않는다.
#[test]
fn retry_recovers_an_exact_contract_and_branch_without_a_worktree() {
    let fixture = Fixture::new("activation-partial");
    let first = fixture.prepare();
    fixture.repository.git([
        "worktree",
        "remove",
        "--force",
        "--",
        first.worktree_path.to_str().unwrap(),
    ]);

    let result = fixture.prepare();

    assert!(matches!(result.effects.contract, Effect::Reused));
    assert!(matches!(result.effects.worktree, Effect::Created));
    assert!(matches!(result.effects.binding, Effect::Created));
}

// 첫 setup이 contract와 branch까지만 남긴 뒤 develop가 전진해도 contract에
// 고정된 최초 base를 복구하여 동일 요청의 worktree와 binding만 완성한다.
#[test]
fn retry_keeps_the_pinned_base_after_develop_advances() {
    let fixture = Fixture::new("activation-advanced-develop");
    let first = fixture.prepare();
    fixture.repository.git([
        "worktree",
        "remove",
        "--force",
        "--",
        first.worktree_path.to_str().unwrap(),
    ]);
    fixture.repository.write("advanced.txt", "advanced\n");
    fixture.repository.git(["add", "advanced.txt"]);
    fixture
        .repository
        .git(["commit", "--quiet", "-m", "test: advance develop"]);

    let result = fixture.prepare();

    assert_eq!(result.base, first.base);
    assert!(matches!(result.effects.contract, Effect::Reused));
    assert!(matches!(result.effects.branch, Effect::Reused));
    assert!(matches!(result.effects.worktree, Effect::Created));
    assert!(matches!(result.effects.binding, Effect::Created));
}

// helper가 소유했다는 exact contract 증거 없이 같은 이름의 branch가 먼저
// 존재하면 이를 인수하지 않고 contract와 worktree를 만들기 전에 거절한다.
#[test]
fn rejects_a_preexisting_branch_without_the_exact_contract() {
    let fixture = Fixture::new("activation-ref-conflict");
    fixture
        .repository
        .git(["branch", &format!("slice/direct/{}", fixture.slice)]);

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("without the exact activation contract"));
    assert!(!fixture.worktree().exists());
}

// exact contract 증거 없이 branch만 남은 상태는 ref가 같은 base여도 helper가
// 준비한 effect로 인수하지 않고 conflicting으로 재조회한다.
#[test]
fn failure_observes_a_branch_only_partial_setup() {
    let fixture = Fixture::new("activation-branch-only-state");
    fixture
        .repository
        .git(["branch", &format!("slice/direct/{}", fixture.slice)]);

    let encoded = run(&fixture.repository.path, &fixture.request).unwrap_err();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["effects"]["contract"]["state"], "absent");
    assert_eq!(failure["effects"]["branch"]["state"], "conflicting");
    assert_eq!(failure["effects"]["worktree"]["state"], "absent");
    assert_eq!(failure["effects"]["binding"]["state"], "unknown");
}

// exact path/ref/base의 worktree가 있어도 owning contract가 사라졌다면 helper가
// 인수하지 않으므로 branch와 worktree를 모두 conflict로 대칭 보고한다.
#[test]
fn failure_observes_a_contractless_worktree_as_conflicting() {
    let fixture = Fixture::new("activation-contractless-worktree");
    let first = fixture.prepare();
    std::fs::remove_file(&first.contract_path).unwrap();

    let encoded = run(&fixture.repository.path, &fixture.request).unwrap_err();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["effects"]["contract"]["state"], "absent");
    assert_eq!(failure["effects"]["branch"]["state"], "conflicting");
    assert_eq!(failure["effects"]["worktree"]["state"], "conflicting");
    assert_eq!(failure["effects"]["binding"]["state"], "unknown");
}

// failure 관찰은 오류 뒤 mutable request path나 움직인 HEAD를 다시 읽지 않고
// invocation 시작 때 캡처한 request bytes와 base를 그대로 보고한다.
#[test]
fn failure_observation_uses_the_invocation_snapshot() {
    let fixture = Fixture::new("activation-failure-snapshot");
    let request = std::fs::read(&fixture.request).unwrap();
    let initial_base = output(&fixture.repository.path, &["rev-parse", "HEAD"]);
    std::fs::write(
        &fixture.request,
        r#"{
  "schema": "yo.activation-slice-request/v1",
  "slice": "different-activation",
  "owned_contracts": ["different.activation"]
}
"#,
    )
    .unwrap();
    fixture.repository.write("advanced.txt", "advanced\n");
    fixture.repository.git(["add", "advanced.txt"]);
    fixture
        .repository
        .git(["commit", "--quiet", "-m", "test: move head"]);

    let failure = observation::failure(
        &fixture.repository.path,
        Some(&request),
        Some(initial_base.clone()),
        "injected failure".to_owned(),
    );

    assert_eq!(failure.slice.as_deref(), Some(fixture.slice.as_str()));
    assert_eq!(failure.base.as_deref(), Some(initial_base.as_str()));
    assert!(
        failure
            .contract_path
            .unwrap()
            .to_string_lossy()
            .contains(&fixture.slice)
    );
}

// request와 다른 bytes의 coordination contract가 있으면 과거 판단을
// 덮어쓰거나 그 위에 worktree를 붙이지 않고 충돌 경로를 보고한다.
#[test]
fn rejects_a_conflicting_coordination_contract() {
    let fixture = Fixture::new("activation-contract-conflict");
    let contract = fixture
        .repository
        .path
        .join(".local-exclude/coordination")
        .join(&fixture.slice)
        .join("slice-contract.json");
    std::fs::create_dir_all(contract.parent().unwrap()).unwrap();
    std::fs::write(&contract, b"{}\n").unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("existing activation Slice contract"));
    assert!(!fixture.worktree().exists());
}

// 중단 뒤 사용자가 수정한 기존 Slice worktree를 exact prepared effect로
// 오인하지 않고 dirty 상태를 보존한 채 binding setup을 중단한다.
#[test]
fn rejects_a_dirty_existing_slice_worktree() {
    let fixture = Fixture::new("activation-dirty-slice");
    fixture.prepare();
    std::fs::write(fixture.worktree().join("untracked.txt"), b"dirty\n").unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("activation Slice worktree must be clean"));
    assert_eq!(
        std::fs::read(fixture.worktree().join("untracked.txt")).unwrap(),
        b"dirty\n"
    );
}

// binding publication과 성공 응답 사이에 worktree가 바뀌면 마지막 postcondition이
// 성공을 거절하고, 이미 만들어진 binding은 structured retry 관찰 대상으로 남긴다.
#[test]
fn rejects_a_worktree_mutation_after_binding() {
    let fixture = Fixture::new("activation-post-binding-mutation");

    let error = prepare_with_post_binding(&fixture.repository.path, &fixture.request, |worktree| {
        std::fs::write(worktree.join("late.txt"), b"late\n").map_err(|write| write.to_string())
    })
    .unwrap_err();

    assert!(error.contains("activation Slice worktree must be clean"));
    let binding = crate::slice_contract::binding_path_for(&fixture.worktree()).unwrap();
    assert!(binding.exists());
}

// exact contract와 worktree가 있어도 binding이 다른 계약을 가리키면 이를
// 덮어쓰지 않고 기존 bytes를 보존하여 Slice 권한 전환을 명시적으로 막는다.
#[test]
fn rejects_and_preserves_a_conflicting_binding() {
    let fixture = Fixture::new("activation-binding-conflict");
    let first = fixture.prepare();
    std::fs::write(&first.binding_path, b"/different/contract.json\n").unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("already contains different bytes"));
    assert_eq!(
        std::fs::read(&first.binding_path).unwrap(),
        b"/different/contract.json\n"
    );
}

// mutation 실패 뒤 CLI error도 parse 가능한 versioned JSON으로 contract,
// branch, worktree, binding의 실제 관찰 상태를 모두 돌려준다.
#[test]
fn failure_reports_every_prepared_or_conflicting_effect() {
    let fixture = Fixture::new("activation-structured-failure");
    let first = fixture.prepare();
    std::fs::write(&first.binding_path, b"/different/contract.json\n").unwrap();

    let encoded = run(&fixture.repository.path, &fixture.request).unwrap_err();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["schema"], "yo.activation-slice-bootstrap/v1");
    assert_eq!(failure["ok"], false);
    assert_eq!(failure["effects"]["contract"]["state"], "prepared");
    assert_eq!(failure["effects"]["branch"]["state"], "prepared");
    assert_eq!(failure["effects"]["worktree"]["state"], "prepared");
    assert_eq!(failure["effects"]["binding"]["state"], "conflicting");
}
