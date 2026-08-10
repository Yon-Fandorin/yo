use super::{
    super::{bind, binding_path_for, check_bound_scope_with_index, trusted_check_bound_scope},
    support::{commit, contract, contract_for_ref},
};
use crate::test_support::TestRepository;

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

    let error = super::super::check_bound_scope(&repository.path).unwrap_err();

    assert!(error.contains("cargo xtask slice-contract bind"));
}
