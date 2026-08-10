use super::{
    super::check_parallel,
    support::{commit, contract},
};
use crate::test_support::TestRepository;

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
