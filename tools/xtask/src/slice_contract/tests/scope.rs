use std::path::Path;

use super::{
    super::check_scope_with_index,
    support::{commit, contract},
};
use crate::test_support::TestRepository;

fn check_scope(repository: &Path, contract_path: &Path) -> Result<(), String> {
    check_scope_with_index(repository, contract_path, None)
}

// 실제 변경 파일이 선언한 하위 트리 안에만 있으면 scope 검사가 통과하여,
// 작업자가 합의된 경계 안에서 구현을 계속할 수 있다.
#[test]
fn scope_accepts_changes_inside_the_declared_write_set() {
    let repository = TestRepository::new("scope-pass");
    repository.write("crates/yo-tui/src/render.rs", "base\n");
    let base = commit(&repository);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    repository.write("crates/yo-tui/src/render.rs", "polished\n");

    check_scope(&repository.path, &contract_path).unwrap();
}

// 선언하지 않은 core 파일이 함께 바뀌면 scope 검사가 실패하여, TUI Slice가
// 저널 소유 경계를 조용히 침범한 채 검수 단계로 넘어가지 못한다.
#[test]
fn scope_rejects_changes_outside_the_declared_write_set() {
    let repository = TestRepository::new("scope-fail");
    repository.write("crates/yo-tui/src/render.rs", "base\n");
    repository.write("crates/yo-core/src/journal.rs", "base\n");
    let base = commit(&repository);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    repository.write("crates/yo-core/src/journal.rs", "changed\n");

    let error = check_scope(&repository.path, &contract_path).unwrap_err();

    assert!(error.contains("crates/yo-core/src/journal.rs"));
}

// 눈으로는 같은 계약 이름 뒤에 공백을 붙여 별도 소유권처럼 보이게 하는
// 입력을 거절하여 contract collision 검사를 우회하지 못하게 한다.
#[test]
fn contract_ownership_rejects_surrounding_whitespace() {
    let repository = TestRepository::new("ownership-whitespace");
    repository.write("README.md", "base\n");
    let base = commit(&repository);
    let invalid = repository.write(
        "invalid.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual "),
    );

    let error = check_scope(&repository.path, &invalid).unwrap_err();

    assert!(error.contains("no surrounding whitespace"));
}

// index의 out-of-scope 변경을 worktree에서 원래 내용으로 되돌려도 두
// 레이어를 따로 관찰하므로 staged 변경이 상쇄되어 숨지 않는다.
#[test]
fn scope_rejects_staged_change_hidden_by_worktree_content() {
    let repository = TestRepository::new("scope-staged-cancellation");
    repository.write("crates/yo-core/src/journal.rs", "base\n");
    let base = commit(&repository);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    repository.write("crates/yo-core/src/journal.rs", "staged\n");
    repository.git(["add", "crates/yo-core/src/journal.rs"]);
    repository.write("crates/yo-core/src/journal.rs", "base\n");

    let error = check_scope(&repository.path, &contract_path).unwrap_err();

    assert!(error.contains("crates/yo-core/src/journal.rs"));
}

// 허용되지 않은 core 파일을 허용된 TUI 트리로 rename해도 rename 추론을
// 끈 diff가 원본 삭제와 목적지 추가를 모두 검사하여 우회를 막는다.
#[test]
fn scope_rejects_rename_from_outside_into_the_allowed_tree() {
    let repository = TestRepository::new("scope-rename-source");
    repository.write("crates/yo-core/src/shared.rs", "base\n");
    repository.write("crates/yo-tui/src/.keep", "base\n");
    let base = commit(&repository);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    repository.git([
        "mv",
        "crates/yo-core/src/shared.rs",
        "crates/yo-tui/src/shared.rs",
    ]);

    let error = check_scope(&repository.path, &contract_path).unwrap_err();

    assert!(error.contains("crates/yo-core/src/shared.rs"));
}

// 계약의 base를 Slice가 만든 새 commit으로 바꾸더라도 그 commit이 선언한
// 통합 branch에 없으면 거절하여 이미 저지른 범위 이탈을 숨기지 못한다.
#[test]
fn scope_rejects_a_base_outside_the_declared_integration_history() {
    let repository = TestRepository::new("scope-untrusted-base");
    repository.write("README.md", "develop\n");
    commit(&repository);
    repository.git(["switch", "--quiet", "-c", "slice/direct/tui-polish"]);
    repository.write("crates/yo-core/src/journal.rs", "out of scope\n");
    let rewritten_base = commit(&repository);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract(
            "tui-polish",
            &rewritten_base,
            "crates/yo-tui/src/**",
            "tui.visual",
        ),
    );

    let error = check_scope(&repository.path, &contract_path).unwrap_err();

    assert!(error.contains("does not belong to integration history"));
}

// assume-unchanged 표시가 붙은 파일은 일반 diff에서 빠질 수 있으므로 scope
// 검사를 중단하여 Git index 표시로 변경을 숨기는 우회를 허용하지 않는다.
#[test]
fn scope_rejects_assume_unchanged_index_entries() {
    let repository = TestRepository::new("scope-assume-unchanged");
    repository.write("crates/yo-core/src/journal.rs", "base\n");
    let base = commit(&repository);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    repository.git([
        "update-index",
        "--assume-unchanged",
        "crates/yo-core/src/journal.rs",
    ]);
    repository.write("crates/yo-core/src/journal.rs", "hidden\n");

    let error = check_scope(&repository.path, &contract_path).unwrap_err();

    assert!(error.contains("assume-unchanged or skip-worktree"));
    assert!(error.contains("crates/yo-core/src/journal.rs"));
}

// skip-worktree 표시도 일반 diff 관찰을 약화하므로 동일하게 거절하여 sparse
// checkout 성격의 index 상태에서는 경계를 확인했다고 잘못 보고하지 않는다.
#[test]
fn scope_rejects_skip_worktree_index_entries() {
    let repository = TestRepository::new("scope-skip-worktree");
    repository.write("crates/yo-core/src/journal.rs", "base\n");
    let base = commit(&repository);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    repository.git([
        "update-index",
        "--skip-worktree",
        "crates/yo-core/src/journal.rs",
    ]);

    let error = check_scope(&repository.path, &contract_path).unwrap_err();

    assert!(error.contains("assume-unchanged or skip-worktree"));
    assert!(error.contains("crates/yo-core/src/journal.rs"));
}

// 하위 디렉터리에서 검사를 실행해도 먼저 저장소 루트를 찾아 전체 worktree를
// 관찰하므로 다른 상위 디렉터리의 범위 밖 untracked 파일을 놓치지 않는다.
#[test]
fn scope_observes_the_whole_repository_from_a_nested_directory() {
    let repository = TestRepository::new("scope-nested-directory");
    repository.write("tools/area/base.rs", "base\n");
    let base = commit(&repository);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tools", &base, "tools/area/**", "tools.area"),
    );
    repository.write("crates/yo-core/src/outside.rs", "outside\n");

    let error =
        check_scope(&repository.path.join("tools/area"), contract_path.as_path()).unwrap_err();

    assert!(error.contains("crates/yo-core/src/outside.rs"));
}
