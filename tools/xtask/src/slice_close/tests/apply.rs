use super::{CloseFixture, git, git_succeeds};
use crate::slice_close::{apply, apply_with_before_delete, identity};

// 계획 이후 develop이 한 커밋이라도 움직이면 apply는 오래된 승인 상태를
// 추측하지 않고 멈추며 Slice worktree와 다른 로컬 자료를 그대로 보존한다.
#[test]
fn rejects_integration_drift_without_cleanup() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    let unrelated = fixture.repository.write("unrelated-note", "keep me\n");
    fixture.repository.git(["add", "unrelated-note"]);
    fixture
        .repository
        .git(["commit", "--quiet", "-m", "test: unrelated later commit"]);
    fixture.write_plan(&plan);

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("refs/heads/develop changed after planning"));
    assert!(fixture.slice_worktree.exists());
    assert!(unrelated.exists());
}

// 최종 삭제 직전에 integration ref가 움직이면 worktree와 binding은 이미 제거됐음을
// 알리고 Slice ref를 보존하여, 별도 검수 없이 branch까지 지우는 일을 막는다.
#[test]
fn final_ref_guard_preserves_the_slice_branch_on_late_integration_drift() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    fixture.write_plan(&plan);

    let error = apply_with_before_delete(&fixture.repository.path, &fixture.plan_path, || {
        fixture.commit_later("late-race.txt");
        Ok(())
    })
    .unwrap_err();

    assert!(error.contains("guarded git update-ref failed"));
    assert!(error.contains("worktree, binding, and standard coordination directory were removed"));
    assert!(error.contains("Slice ref refs/heads/slice/direct/sample"));
    assert!(error.contains("separately verified branch cleanup"));
    assert!(!fixture.slice_worktree.exists());
    assert!(!fixture.contract_path.exists());
    assert!(git_succeeds(
        &fixture.repository.path,
        &["show-ref", "--verify", "refs/heads/slice/direct/sample"]
    ));
}

// plan JSON의 한 필드라도 수동 수정하면 해시가 달라지므로 apply는 Git 정리를
// 시작하기 전에 멈추고 기존 worktree와 branch를 그대로 보존한다.
#[test]
fn rejects_a_tampered_plan_before_cleanup() {
    let fixture = CloseFixture::new();
    let mut plan = fixture.plan();
    plan.slice_base = plan.slice_head.clone();
    fixture.write_plan(&plan);

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("identity mismatch"));
    assert!(fixture.slice_worktree.exists());
}

// caller가 coordination-contract 삭제 효과를 바꾸고 plan hash까지 다시 계산해도
// apply는 표준 경로에서 효과를 독립적으로 재도출하여 cleanup 전에 거절한다.
#[test]
fn rejects_a_rehashed_but_incorrect_contract_effect() {
    let fixture = CloseFixture::new();
    let mut plan = fixture.plan();
    plan.effects.remove_coordination_contract = false;
    plan.effects.remove_coordination_directory = false;
    plan.coordination_cleanup_paths.clear();
    plan.retained_coordination_paths =
        vec![fixture.metrics_path.clone(), fixture.contract_path.clone()];
    plan.plan_id = identity(&plan).unwrap();
    fixture.write_plan(&plan);

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("effect does not match"));
    assert!(fixture.slice_worktree.exists());
    assert!(fixture.contract_path.exists());
}

// plan 이후 contract의 허용 범위 같은 내용이 바뀌면 경로와 핵심 필드가 같아도
// contract hash가 달라져 apply가 오래된 검수 근거로 worktree를 지우지 않는다.
#[test]
fn rejects_contract_content_drift() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    fixture.write_plan(&plan);
    let contract = std::fs::read_to_string(&fixture.contract_path).unwrap();
    std::fs::write(
        &fixture.contract_path,
        contract.replace("test focused", "test focused changed"),
    )
    .unwrap();

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("binding or contract changed"));
    assert!(fixture.slice_worktree.exists());
}

// plan을 검토한 뒤 coordination 항목이 추가되거나 사라지면 apply는 사용자가 본
// cleanup 목록과 현재 상태가 다르므로 어떤 삭제도 시작하기 전에 거절합니다.
#[test]
fn rejects_coordination_drift_before_cleanup() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    fixture.write_plan(&plan);
    let handoff = fixture.contract_path.parent().unwrap().join("handoff.md");
    std::fs::write(&handoff, "new after plan\n").unwrap();

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("Slice coordination cleanup paths changed"));
    assert!(fixture.slice_worktree.exists());
    assert!(fixture.contract_path.exists());
    assert!(handoff.exists());
}

// 기존 coordination 하위 디렉터리 안에 계획 이후 파일이 추가돼도 재귀 경로 목록이
// 변화를 감지하여 새 파일과 Slice worktree를 모두 보존합니다.
#[test]
fn rejects_nested_coordination_drift_before_cleanup() {
    let fixture = CloseFixture::new();
    let notes = fixture.contract_path.with_file_name("notes");
    std::fs::create_dir(&notes).unwrap();
    let plan = fixture.plan();
    fixture.write_plan(&plan);
    let late = notes.join("late.txt");
    std::fs::write(&late, "late\n").unwrap();

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("Slice coordination cleanup paths changed"));
    assert!(fixture.slice_worktree.exists());
    assert!(late.exists());
}

// 계획된 경로의 worktree가 detached되거나 다른 branch로 전환된 것은 제거 완료가
// 아니므로 apply가 두 상태 모두 거절하고 원래 Slice ref를 보존한다.
#[test]
fn rejects_detached_or_switched_planned_worktrees() {
    for switch in [
        vec!["switch", "--quiet", "--detach"],
        vec!["switch", "--quiet", "-c", "unrelated"],
    ] {
        let fixture = CloseFixture::new();
        let plan = fixture.plan();
        fixture.write_plan(&plan);
        git(&fixture.slice_worktree, &switch);

        let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

        assert!(error.contains("still registered with different state"));
        assert!(git_succeeds(
            &fixture.repository.path,
            &["show-ref", "--verify", "refs/heads/slice/direct/sample"]
        ));
    }
}

// 등록만 사라지고 계획 경로나 binding이 남은 상태는 apply 중단의 증거가
// 아니므로 branch 삭제를 거절하고 사람이 원인을 확인할 수 있게 보존한다.
#[test]
fn rejects_missing_registration_when_the_planned_path_still_exists() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    fixture.write_plan(&plan);
    git(
        &fixture.repository.path,
        &[
            "worktree",
            "remove",
            "--",
            fixture.slice_worktree.to_str().unwrap(),
        ],
    );
    std::fs::create_dir_all(&fixture.slice_worktree).unwrap();

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("path or binding still exists"));
    assert!(git_succeeds(
        &fixture.repository.path,
        &["show-ref", "--verify", "refs/heads/slice/direct/sample"]
    ));
}

// plan을 삭제 대상 worktree 안에 두면 apply 자체가 그 검토 기록을 지우므로
// 경로 내부 여부를 먼저 확인해 정리 전에 명시적으로 거절한다.
#[test]
fn rejects_a_plan_stored_inside_the_target_worktree() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    let inside = fixture.slice_worktree.join("close-plan.json");
    std::fs::write(&inside, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();

    let error = apply(&fixture.repository.path, &inside).unwrap_err();

    assert!(error.contains("outside the worktree"));
    assert!(fixture.slice_worktree.exists());
}

// 외부에서 v3 plan을 coordination 디렉터리 안에 복사해도 apply는 그 파일을
// 보존 목록 drift로 오인하지 않고 금지된 저장 위치를 직접 진단한다.
#[test]
fn rejects_a_plan_stored_inside_the_slice_coordination_directory() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    let inside = fixture.contract_path.with_file_name("close-plan.json");
    std::fs::write(&inside, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();

    let error = apply(&fixture.repository.path, &inside).unwrap_err();

    assert!(error.contains("outside the worktree and coordination directory"));
    assert!(fixture.slice_worktree.exists());
    assert!(fixture.contract_path.exists());
}

// 정상 apply는 등록된 worktree와 exact 표준 contract를 제거한 뒤 integration ref
// 확인과 예상 Slice SHA 삭제를 한 transaction으로 묶어 Git의 "미병합" 판정에
// 의존하지 않는다.
#[test]
fn completes_the_verified_cleanup() {
    let fixture = CloseFixture::new();
    let coordination = fixture.contract_path.parent().unwrap().to_path_buf();
    let plan = fixture.plan();
    fixture.write_plan(&plan);

    apply(&fixture.repository.path, &fixture.plan_path).unwrap();

    assert!(!fixture.slice_worktree.exists());
    assert!(!fixture.contract_path.exists());
    assert!(!coordination.exists());
    assert!(!git_succeeds(
        &fixture.repository.path,
        &["show-ref", "--verify", "refs/heads/slice/direct/sample"]
    ));
}

// 정상 cleanup은 plan에 보고된 완료 Slice의 coordination 파일과 하위 디렉터리를
// 표준 contract와 함께 제거하여 별도의 수동 캐시 정리를 남기지 않습니다.
#[test]
fn removes_reported_coordination_paths() {
    let fixture = CloseFixture::new();
    let coordination = fixture.contract_path.parent().unwrap();
    let handoff = coordination.join("handoff.md");
    let notes = coordination.join("notes");
    std::fs::write(&handoff, "retain me\n").unwrap();
    std::fs::create_dir(&notes).unwrap();
    let plan = fixture.plan();
    fixture.write_plan(&plan);

    apply(&fixture.repository.path, &fixture.plan_path).unwrap();

    assert!(!handoff.exists());
    assert!(!notes.exists());
    assert!(!fixture.contract_path.exists());
    assert!(!coordination.exists());
}

#[cfg(unix)]
// coordination 내부 symlink는 링크 항목만 삭제하고 가리키는 외부 디렉터리는 따라가지
// 않아 완료 캐시 정리가 Slice 경계 밖의 로컬 자료를 제거하지 않습니다.
#[test]
fn coordination_cleanup_does_not_follow_symlinks() {
    let fixture = CloseFixture::new();
    let outside = crate::test_support::unique_path("slice-close-outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("preserved.txt"), "preserve me\n").unwrap();
    std::os::unix::fs::symlink(
        &outside,
        fixture.contract_path.with_file_name("outside-link"),
    )
    .unwrap();
    let plan = fixture.plan();
    fixture.write_plan(&plan);

    apply(&fixture.repository.path, &fixture.plan_path).unwrap();

    assert!(outside.join("preserved.txt").exists());
    std::fs::remove_dir_all(outside).unwrap();
}

// v3 도입 전에 발행된 v2 plan은 새 retained 필드가 없어도 기존 identity로
// 검증되어 중단된 cleanup을 다시 시작할 수 있다.
#[test]
fn applies_a_legacy_v2_plan() {
    let fixture = CloseFixture::new();
    let mut plan = fixture.plan();
    plan.schema = "yo.slice-close-plan/v2".to_owned();
    plan.close_metrics = None;
    plan.retained_coordination_paths.clear();
    plan.coordination_cleanup_paths.clear();
    plan.effects.remove_coordination_directory = false;
    plan.plan_id = identity(&plan).unwrap();
    let mut encoded = serde_json::to_value(&plan).unwrap();
    encoded
        .as_object_mut()
        .unwrap()
        .remove("retained_coordination_paths");
    std::fs::write(
        &fixture.plan_path,
        serde_json::to_vec_pretty(&encoded).unwrap(),
    )
    .unwrap();

    apply(&fixture.repository.path, &fixture.plan_path).unwrap();

    assert!(!fixture.slice_worktree.exists());
    assert!(!fixture.contract_path.exists());
}

// v2 identity가 알지 못하는 새 필드를 끼워 넣어도 검증되지 않은 값이 plan의
// 일부인 것처럼 보이지 않도록 cleanup 전에 명시적으로 거절한다.
#[test]
fn rejects_retained_paths_added_to_a_legacy_plan() {
    let fixture = CloseFixture::new();
    let mut plan = fixture.plan();
    plan.schema = "yo.slice-close-plan/v2".to_owned();
    plan.close_metrics = None;
    plan.retained_coordination_paths = vec![fixture.contract_path.with_file_name("handoff.md")];
    plan.coordination_cleanup_paths.clear();
    plan.effects.remove_coordination_directory = false;
    plan.plan_id = identity(&plan).unwrap();
    fixture.write_plan(&plan);

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("legacy Slice close plans cannot contain"));
    assert!(fixture.slice_worktree.exists());
}

// v1alpha1에서 metrics를 빼거나 v3에 새 metrics 필드를 붙이면 어느 identity 규칙으로
// 검증할지 모호하므로 apply가 cleanup 전에 두 schema/field 조합을 모두 거부한다.
#[test]
fn rejects_close_metrics_that_do_not_match_the_plan_schema() {
    let current = CloseFixture::new();
    let mut missing = current.plan();
    missing.close_metrics = None;
    current.write_plan(&missing);
    let missing_error = apply(&current.repository.path, &current.plan_path).unwrap_err();
    assert!(missing_error.contains("v1alpha1 or v4 requires bound close metrics"));
    assert!(current.slice_worktree.exists());

    let legacy = CloseFixture::new();
    let mut unexpected = legacy.plan();
    unexpected.schema = "yo.slice-close-plan/v3".to_owned();
    unexpected.coordination_cleanup_paths.clear();
    unexpected.effects.remove_coordination_directory = false;
    legacy.write_plan(&unexpected);
    let unexpected_error = apply(&legacy.repository.path, &legacy.plan_path).unwrap_err();
    assert!(unexpected_error.contains("plans before v4 cannot contain close metrics"));
    assert!(legacy.slice_worktree.exists());
}

// worktree 제거 직후 프로세스가 중단된 경우에도 계획된 경로와 binding이 사라지고
// Slice ref가 같은 SHA로 남았을 때만 재실행이 branch 삭제를 마친다.
#[test]
fn resumes_after_the_planned_worktree_was_already_removed() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    fixture.write_plan(&plan);
    git(
        &fixture.repository.path,
        &[
            "worktree",
            "remove",
            "--",
            fixture.slice_worktree.to_str().unwrap(),
        ],
    );

    apply(&fixture.repository.path, &fixture.plan_path).unwrap();

    assert!(!fixture.contract_path.exists());
    assert!(!git_succeeds(
        &fixture.repository.path,
        &["show-ref", "--verify", "refs/heads/slice/direct/sample"]
    ));
}

// 표준 coordination 경로가 아닌 contract는 plan에 삭제 효과가 없으며,
// worktree와 branch를 닫아도 caller가 둔 외부 파일을 보존한다.
#[test]
fn preserves_a_nonstandard_contract_path() {
    let fixture = CloseFixture::new();
    let external = crate::test_support::unique_path("slice-close-external-contract.json");
    std::fs::copy(&fixture.contract_path, &external).unwrap();
    crate::slice_contract::bind(&fixture.slice_worktree, &external).unwrap();
    let plan = fixture.plan();
    fixture.write_plan(&plan);

    assert!(!plan.effects.remove_coordination_contract);
    apply(&fixture.repository.path, &fixture.plan_path).unwrap();

    assert!(external.exists());
    std::fs::remove_file(external).unwrap();
}

// worktree와 표준 contract가 이미 제거된 중단 상태에서도 plan의 exact ref가
// 유지되면 재실행은 없는 contract를 오류로 보지 않고 branch 삭제를 수렴한다.
#[test]
fn resumes_after_worktree_and_contract_were_already_removed() {
    let fixture = CloseFixture::new();
    let mut plan = fixture.plan();
    plan.schema = "yo.slice-close-plan/v4".to_owned();
    plan.retained_coordination_paths = vec![fixture.metrics_path.clone()];
    plan.coordination_cleanup_paths.clear();
    plan.effects.remove_coordination_directory = false;
    plan.plan_id = identity(&plan).unwrap();
    fixture.write_plan(&plan);
    git(
        &fixture.repository.path,
        &[
            "worktree",
            "remove",
            "--",
            fixture.slice_worktree.to_str().unwrap(),
        ],
    );
    std::fs::remove_file(&fixture.contract_path).unwrap();

    apply(&fixture.repository.path, &fixture.plan_path).unwrap();

    assert!(!git_succeeds(
        &fixture.repository.path,
        &["show-ref", "--verify", "refs/heads/slice/direct/sample"]
    ));
}

// worktree 제거 뒤 contract bytes가 바뀐 중단 상태에서는 plan hash와 다른 로컬
// 판단을 지우거나 branch를 닫지 않고 exact mismatch로 중단한다.
#[test]
fn changed_contract_after_worktree_removal_is_preserved() {
    let fixture = CloseFixture::new();
    let mut plan = fixture.plan();
    plan.schema = "yo.slice-close-plan/v4".to_owned();
    plan.retained_coordination_paths = vec![fixture.metrics_path.clone()];
    plan.coordination_cleanup_paths.clear();
    plan.effects.remove_coordination_directory = false;
    plan.plan_id = identity(&plan).unwrap();
    fixture.write_plan(&plan);
    git(
        &fixture.repository.path,
        &[
            "worktree",
            "remove",
            "--",
            fixture.slice_worktree.to_str().unwrap(),
        ],
    );
    std::fs::write(&fixture.contract_path, b"changed after interruption\n").unwrap();

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("hash changed"));
    assert!(fixture.contract_path.exists());
    assert!(git_succeeds(
        &fixture.repository.path,
        &["show-ref", "--verify", "refs/heads/slice/direct/sample"]
    ));
}
