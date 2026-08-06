use super::{CloseFixture, git, output};
use crate::{
    slice_close::{build_plan, identity},
    test_support,
};

// plan은 승인된 커밋과 Slice 패치가 같고 양쪽 worktree가 깨끗할 때만
// 실제 제거 대상·고정된 Git 상태·세 가지 제한된 효과를 해시로 묶는다.
#[test]
fn describes_only_the_verified_slice_cleanup() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();

    assert_eq!(plan.schema, "yo.slice-close-plan/v1");
    assert_eq!(plan.slice, "sample");
    assert_eq!(plan.integration_ref, "refs/heads/develop");
    assert_eq!(plan.integration_head, plan.accepted_commit);
    assert_eq!(plan.worktree_path, fixture.slice_worktree);
    assert!(plan.effects.remove_worktree);
    assert!(plan.effects.remove_binding);
    assert!(plan.effects.delete_slice_branch);
    assert_eq!(plan.plan_id, identity(&plan).unwrap());
}

// 이후 Slice가 develop에 들어와 HEAD가 움직여도 first-parent 이력에서 patch가
// 정확히 같은 이전 수용 commit을 찾아 오래된 로컬 Slice를 닫을 수 있게 한다.
#[test]
fn finds_the_accepted_commit_beneath_a_later_integration_head() {
    let fixture = CloseFixture::new();
    let accepted = output(&fixture.repository.path, &["rev-parse", "HEAD"]);
    fixture.commit_later("later.txt");

    let plan = fixture.plan();

    assert_eq!(plan.accepted_commit, accepted);
    assert_ne!(plan.integration_head, plan.accepted_commit);
}

// 수용된 Slice 뒤에 내용이 없는 이정표 commit이 추가돼도 빈 patch를 후보에서
// 건너뛰고 실제 Slice patch를 가진 이전 수용 commit을 계속 찾는다.
#[test]
fn skips_empty_commits_while_scanning_integration_history() {
    let fixture = CloseFixture::new();
    let accepted = output(&fixture.repository.path, &["rev-parse", "HEAD"]);
    fixture.repository.git([
        "commit",
        "--allow-empty",
        "--quiet",
        "-m",
        "chore: empty milestone",
    ]);

    let plan = fixture.plan();

    assert_eq!(plan.accepted_commit, accepted);
}

// direct와 Wave에 같은 leaf 이름의 Slice가 함께 있어도 현재 integration ref가
// 소유한 정확한 direct branch만 골라 unrelated Wave worktree와 충돌하지 않는다.
#[test]
fn selects_the_slice_owned_by_the_current_integration_ref() {
    let fixture = CloseFixture::new();
    let wave_worktree = test_support::unique_path("slice-close-same-name-wave");
    fixture.repository.git(["branch", "slice/w1/sample"]);
    fixture.repository.git([
        "worktree",
        "add",
        "--quiet",
        wave_worktree.to_str().unwrap(),
        "slice/w1/sample",
    ]);

    let plan = fixture.plan();

    assert_eq!(plan.slice_ref, "refs/heads/slice/direct/sample");
    git(
        &fixture.repository.path,
        &["worktree", "remove", "--", wave_worktree.to_str().unwrap()],
    );
}

// Wave integration worktree에서는 동일한 leaf 이름을 direct로 오인하지 않고
// wave 이름이 포함된 정확한 Slice ref와 base_ref를 plan에 기록한다.
#[test]
fn derives_the_exact_wave_slice_ref() {
    let fixture = CloseFixture::new_wave();

    let plan = fixture.plan();

    assert_eq!(plan.integration_ref, "refs/heads/wave/w1");
    assert_eq!(plan.slice_ref, "refs/heads/slice/w1/sample");
}

// integration이나 Slice worktree에 tracked 또는 일반 untracked 변경이 있으면
// 검토한 snapshot이 아니므로 plan은 삭제 계획을 만들기 전에 중단한다.
#[test]
fn rejects_dirty_integration_and_slice_worktrees() {
    let integration = CloseFixture::new();
    integration.repository.write("dirty.txt", "dirty\n");
    let integration_error = build_plan(&integration.repository.path, "sample").unwrap_err();
    assert!(integration_error.contains("integration worktree must be clean"));

    let slice = CloseFixture::new();
    std::fs::write(slice.slice_worktree.join("dirty.txt"), "dirty\n").unwrap();
    let slice_error = build_plan(&slice.repository.path, "sample").unwrap_err();
    assert!(slice_error.contains("Slice worktree must be clean"));
}

// Slice가 수용 뒤 더 수정되거나 수용 commit의 검수 trailer가 사라지면 patch와
// review 근거가 달라지므로 plan 생성이 각각 실패한다.
#[test]
fn rejects_patch_or_review_evidence_drift() {
    let patch = CloseFixture::new();
    std::fs::write(patch.slice_worktree.join("feature.txt"), "changed again\n").unwrap();
    git(&patch.slice_worktree, &["add", "feature.txt"]);
    git(
        &patch.slice_worktree,
        &["commit", "--quiet", "-m", "feat: changed after acceptance"],
    );
    let patch_error = build_plan(&patch.repository.path, "sample").unwrap_err();
    assert!(patch_error.contains("no accepted commit"));

    let review = CloseFixture::new();
    review.repository.git([
        "commit",
        "--amend",
        "--quiet",
        "-m",
        "feat: no review evidence",
    ]);
    let review_error = build_plan(&review.repository.path, "sample").unwrap_err();
    assert!(review_error.contains("invalid review evidence"));
}
