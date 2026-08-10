use std::path::Path;

use super::{
    PathRule, bind, binding_path_for, check_bound_scope, check_bound_scope_with_index,
    check_parallel, check_scope_with_index, overlaps, trusted_check_bound_scope,
};
use crate::{git, test_support::TestRepository};

fn check_scope(repository: &Path, contract_path: &Path) -> Result<(), String> {
    check_scope_with_index(repository, contract_path, None)
}

// review publish 시점의 silent trusted guard가 bound Slice write-set 밖에 commit된
// 경로를 발견하면 거부함을 확인한다.
#[test]
fn trusted_bound_scope_rejects_a_committed_outside_path() {
    let repository = TestRepository::new("trusted-review-scope");
    repository.write("allowed.txt", "base\n");
    let base = commit(&repository);
    repository.git(["switch", "-c", "slice/direct/trusted-review-scope"]);
    let contract_path = repository.write(
        ".local-exclude/contract.json",
        &contract(
            "trusted-review-scope",
            &base,
            "allowed.txt",
            "trusted review scope",
        ),
    );
    bind(&repository.path, &contract_path).unwrap();
    repository.write("outside.txt", "not leased\n");
    repository.git(["add", "outside.txt"]);
    repository.git(["commit", "--quiet", "-m", "outside"]);

    assert!(
        trusted_check_bound_scope(&repository.path)
            .unwrap_err()
            .contains("outside its allowed write-set")
    );
}

fn commit(repository: &TestRepository) -> String {
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
        .unwrap()
        .trim()
        .to_owned()
}

fn contract(slice: &str, base: &str, path: &str, contract: &str) -> String {
    contract_for_ref(slice, base, "refs/heads/develop", path, contract)
}

fn contract_for_ref(slice: &str, base: &str, base_ref: &str, path: &str, contract: &str) -> String {
    format!(
        r#"{{
  "schema": "yo.slice-contract/v1",
  "slice": "{slice}",
  "base": "{base}",
  "base_ref": "{base_ref}",
  "owned_contracts": ["{contract}"],
  "dependencies": [],
  "allowed_write_set": ["{path}"],
  "focused_checks": ["cargo test -p owner"],
  "slice_close_checks": ["hk check"]
}}"#
    )
}

// 디렉터리 규칙은 그 디렉터리 자체와 하위 파일을 허용하지만 이름 접두사가
// 우연히 같은 이웃 디렉터리까지 허용하지 않아 Slice 경계를 넘지 않는다.
#[test]
fn tree_rule_matches_only_its_repository_subtree() {
    let rule = PathRule::parse("crates/yo-tui/src/**").unwrap();

    assert!(rule.matches("crates/yo-tui/src"));
    assert!(rule.matches("crates/yo-tui/src/render/mod.rs"));
    assert!(!rule.matches("crates/yo-tui/src-old/render.rs"));
}

// 한 Slice의 디렉터리 lease가 다른 Slice의 파일 lease를 포함하면 실제
// 변경 전이라도 병렬 충돌로 찾아 공용 파일의 이중 writer를 막는다.
#[test]
fn overlap_detects_a_file_inside_another_slice_tree() {
    let left = [PathRule::parse("crates/yo-tui/src/**").unwrap()];
    let right = [PathRule::parse("crates/yo-tui/src/lib.rs").unwrap()];

    assert_eq!(overlaps(&left, &right).len(), 1);
}

// 서로 다른 crate의 하위 트리는 같은 파일을 쓸 수 없으므로 병렬 실행
// 가능 대상으로 유지하고 불필요하게 저장소 전체를 직렬화하지 않는다.
#[test]
fn disjoint_crate_trees_can_run_in_parallel() {
    let left = [PathRule::parse("crates/yo-tui/src/**").unwrap()];
    let right = [PathRule::parse("crates/yo-core/src/journal/**").unwrap()];

    assert!(overlaps(&left, &right).is_empty());
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

// planner가 Slice branch에 계약을 한 번 bind하면 새 agent session은 경로를
// 전달받지 않아도 worktree-local 포인터를 따라 같은 scope 검사를 실행한다.
#[test]
fn bound_contract_is_discovered_without_a_path_argument() {
    let repository = TestRepository::new("scope-bound-contract");
    repository.write("crates/yo-tui/src/render.rs", "base\n");
    let base = commit(&repository);
    repository.git(["switch", "--quiet", "-c", "slice/direct/tui-polish"]);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );

    bind(&repository.path, &contract_path).unwrap();
    repository.write("crates/yo-tui/src/render.rs", "polished\n");

    check_bound_scope_with_index(&repository.path, None).unwrap();
}

// 명시적 일반 bind 명령은 planner가 검증한 새 계약 경로로 기존 binding을
// 교체할 수 있어 stale pointer에서 회복하는 종전 workflow를 보존한다.
#[test]
fn explicit_bind_replaces_a_stale_binding() {
    let repository = TestRepository::new("scope-rebind-contract");
    repository.write("crates/yo-tui/src/render.rs", "base\n");
    let base = commit(&repository);
    repository.git(["switch", "--quiet", "-c", "slice/direct/tui-polish"]);
    let first = repository.write(
        ".git/first-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    let second = repository.write(
        ".git/second-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );

    bind(&repository.path, &first).unwrap();
    bind(&repository.path, &second).unwrap();

    let binding = binding_path_for(&repository.path).unwrap();
    assert_eq!(
        std::fs::read_to_string(binding).unwrap(),
        format!("{}\n", std::fs::canonicalize(second).unwrap().display())
    );
}

// 현재 branch와 다른 Slice 계약은 bind 단계에서 거절하여 새 agent가
// 이름이 비슷한 다른 작업의 write-set을 자신의 경계로 오인하지 않는다.
#[test]
fn binding_rejects_a_contract_for_another_slice_branch() {
    let repository = TestRepository::new("scope-wrong-binding");
    repository.write("README.md", "base\n");
    let base = commit(&repository);
    repository.git(["switch", "--quiet", "-c", "slice/direct/other"]);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );

    let error = bind(&repository.path, &contract_path).unwrap_err();

    assert!(error.contains("does not match current Slice or Task branch"));
}

// bind 뒤 같은 worktree를 다른 branch로 전환해도 매 시작 검사가 현재
// branch를 다시 대조하므로 이전 Slice의 권한을 계속 사용할 수 없다.
#[test]
fn bound_contract_is_rejected_after_switching_to_another_slice() {
    let repository = TestRepository::new("scope-stale-binding");
    repository.write("README.md", "base\n");
    let base = commit(&repository);
    repository.git(["switch", "--quiet", "-c", "slice/direct/tui-polish"]);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    bind(&repository.path, &contract_path).unwrap();
    repository.git(["switch", "--quiet", "-c", "slice/direct/other"]);

    let error = check_bound_scope_with_index(&repository.path, None).unwrap_err();

    assert!(error.contains("does not match current Slice or Task branch"));
}

// leaf worker가 쓰는 direct Task branch도 부모 Slice의 동일한 계약을
// bind할 수 있어 새 coding-agent session의 시작 검사를 수행할 수 있다.
#[test]
fn direct_task_branch_accepts_its_parent_slice_contract() {
    let repository = TestRepository::new("scope-direct-task");
    repository.write("README.md", "base\n");
    let base = commit(&repository);
    repository.git([
        "switch",
        "--quiet",
        "-c",
        "task/direct/tui-polish/rendering",
    ]);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );

    bind(&repository.path, &contract_path).unwrap();
    check_bound_scope_with_index(&repository.path, None).unwrap();
}

// Wave 이름이 branch와 계약의 base_ref에서 다르면 Slice 이름이 같아도
// 서로 다른 통합선이므로 bind 단계에서 명확하게 거절한다.
#[test]
fn wave_branch_rejects_a_contract_from_another_wave() {
    let repository = TestRepository::new("scope-wrong-wave");
    repository.write("README.md", "base\n");
    let base = commit(&repository);
    repository.git(["branch", "wave/w1"]);
    repository.git(["switch", "--quiet", "-c", "slice/w2/tui-polish"]);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract_for_ref(
            "tui-polish",
            &base,
            "refs/heads/wave/w1",
            "crates/yo-tui/src/**",
            "tui.visual",
        ),
    );

    let error = bind(&repository.path, &contract_path).unwrap_err();

    assert!(error.contains("belongs to `refs/heads/wave/w2`"));
}

// Wave Task branch는 동일한 Wave와 부모 Slice를 선언한 계약만 받아
// 병렬 worker가 올바른 통합선의 write-set을 공유하도록 한다.
#[test]
fn wave_task_branch_accepts_its_parent_slice_contract() {
    let repository = TestRepository::new("scope-wave-task");
    repository.write("README.md", "base\n");
    let base = commit(&repository);
    repository.git(["branch", "wave/w1"]);
    repository.git(["switch", "--quiet", "-c", "task/w1/tui-polish/rendering"]);
    let contract_path = repository.write(
        ".git/slice-contract.json",
        &contract_for_ref(
            "tui-polish",
            &base,
            "refs/heads/wave/w1",
            "crates/yo-tui/src/**",
            "tui.visual",
        ),
    );

    bind(&repository.path, &contract_path).unwrap();
    check_bound_scope_with_index(&repository.path, None).unwrap();
}

// 아직 계약을 bind하지 않은 worktree에서는 임의의 기본 범위를 추측하지
// 않고 planner가 실행할 명령을 알려줘 경계 없는 구현 시작을 막는다.
#[test]
fn missing_binding_fails_with_a_recovery_command() {
    let repository = TestRepository::new("scope-missing-binding");

    let error = check_bound_scope(&repository.path).unwrap_err();

    assert!(error.contains("cargo xtask slice-contract bind"));
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

// 같은 develop commit에서 출발하고 write-set과 계약 소유권이 분리된 두
// Slice는 실제 구현 전에 병렬 가능 판정을 받는다.
#[test]
fn parallel_check_accepts_disjoint_slices_on_current_develop() {
    let repository = TestRepository::new("parallel-pass");
    repository.write("README.md", "base\n");
    let base = commit(&repository);
    let tui = repository.write(
        "tui.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    let journal = repository.write(
        "journal.json",
        &contract(
            "journal-codec",
            &base,
            "crates/yo-core/src/journal/**",
            "agent.journal",
        ),
    );

    check_parallel(&repository.path, &tui, &journal).unwrap();
}

// 두 Slice가 같은 TUI 트리의 상위·하위 범위를 각각 선언하면 병렬 검사가
// 시작 전에 거절하여 나중의 merge conflict에 의존하지 않는다.
#[test]
fn parallel_check_rejects_overlapping_write_sets() {
    let repository = TestRepository::new("parallel-fail");
    repository.write("README.md", "base\n");
    let base = commit(&repository);
    let whole_tui = repository.write(
        "whole.json",
        &contract("whole-tui", &base, "crates/yo-tui/src/**", "tui.visual"),
    );
    let appearance = repository.write(
        "appearance.json",
        &contract(
            "appearance",
            &base,
            "crates/yo-tui/src/appearance/**",
            "tui.appearance",
        ),
    );

    let error = check_parallel(&repository.path, &whole_tui, &appearance).unwrap_err();

    assert!(error.contains("overlapping write leases"));
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

// Wave에서 이미 수용된 Slice 뒤에 시작하는 병렬 Slice들은 develop이
// 아니라 현재 Wave commit을 공통 기준으로 삼아도 사전검사를 통과한다.
#[test]
fn parallel_check_accepts_a_current_wave_integration_base() {
    let repository = TestRepository::new("parallel-wave-base");
    repository.write("README.md", "develop\n");
    commit(&repository);
    repository.git(["switch", "--quiet", "-c", "wave/w1-runtime"]);
    repository.write("accepted.txt", "accepted Slice\n");
    let base = commit(&repository);
    let tui = repository.write(
        "tui.json",
        &contract("tui-polish", &base, "crates/yo-tui/src/**", "tui.visual")
            .replace("refs/heads/develop", "refs/heads/wave/w1-runtime"),
    );
    let journal = repository.write(
        "journal.json",
        &contract(
            "journal-codec",
            &base,
            "crates/yo-core/src/journal/**",
            "agent.journal",
        )
        .replace("refs/heads/develop", "refs/heads/wave/w1-runtime"),
    );

    check_parallel(&repository.path, &tui, &journal).unwrap();
}
