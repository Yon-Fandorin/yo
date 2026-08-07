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
    assert!(error.contains("worktree, binding, and standard coordination contract were removed"));
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

// 정상 apply는 등록된 worktree와 exact 표준 contract를 제거한 뒤 integration ref
// 확인과 예상 Slice SHA 삭제를 한 transaction으로 묶어 Git의 "미병합" 판정에
// 의존하지 않는다.
#[test]
fn completes_the_verified_cleanup() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    fixture.write_plan(&plan);

    apply(&fixture.repository.path, &fixture.plan_path).unwrap();

    assert!(!fixture.slice_worktree.exists());
    assert!(!fixture.contract_path.exists());
    assert!(!git_succeeds(
        &fixture.repository.path,
        &["show-ref", "--verify", "refs/heads/slice/direct/sample"]
    ));
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
    std::fs::write(&fixture.contract_path, b"changed after interruption\n").unwrap();

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("hash changed"));
    assert!(fixture.contract_path.exists());
    assert!(git_succeeds(
        &fixture.repository.path,
        &["show-ref", "--verify", "refs/heads/slice/direct/sample"]
    ));
}
