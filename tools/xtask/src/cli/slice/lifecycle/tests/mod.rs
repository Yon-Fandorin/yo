use super::{
    activation_slice_usage, slice_accept_usage, slice_close_usage, slice_commit_usage,
    slice_create_usage, slice_gate_usage, slice_status_usage,
};
use crate::cli::run;

// 일반 Slice bootstrap도 정확히 한 immutable 계약만 받아 누락되거나 추가된
// positional input이 별도 branch identity로 해석되지 않게 합니다.
#[test]
fn slice_create_requires_exactly_one_contract() {
    let missing = run(["slice", "create"].map(Into::into)).unwrap_err();
    let extra =
        run(["slice", "create", "slice-contract.json", "extra.json"].map(Into::into)).unwrap_err();

    assert_eq!(missing, slice_create_usage());
    assert_eq!(extra, slice_create_usage());
}

// activation Slice 생성은 정확히 한 versioned request만 받아 누락되거나
// 조용히 무시된 추가 입력으로 branch와 worktree를 만들지 않는다.
#[test]
fn activation_slice_requires_exactly_one_request() {
    let missing = run(["slice", "create-activation"].map(Into::into)).unwrap_err();
    let extra = run(["slice", "create-activation", "request.json", "extra.json"].map(Into::into))
        .unwrap_err();

    assert_eq!(missing, activation_slice_usage());
    assert_eq!(extra, activation_slice_usage());
}

// status와 accept도 Slice 이름 또는 versioned request 하나만 받아 compact 관측과
// mutation 경계가 여분 인자로 달라지지 않으며, prepare 역시 입력 하나만 받습니다.
#[test]
fn status_and_accept_require_one_input() {
    for (scope, usage) in [
        ("status", slice_status_usage()),
        ("accept", slice_accept_usage()),
    ] {
        assert_eq!(run(["slice", scope].map(Into::into)).unwrap_err(), usage);
        assert_eq!(
            run(["slice", scope, "input", "extra"].map(Into::into)).unwrap_err(),
            usage
        );
    }
    assert_eq!(
        run(["slice", "accept", "prepare"].map(Into::into)).unwrap_err(),
        slice_accept_usage()
    );
    assert_eq!(
        run(["slice", "accept", "prepare", "input", "extra"].map(Into::into)).unwrap_err(),
        slice_accept_usage()
    );
}

// Slice gate는 한 후보에 결속된 단일 request만 받아 서로 다른 후보의 증거가
// 추가 인자로 섞이거나 request 없는 기본 동작으로 승인되지 않게 한다.
#[test]
fn slice_gate_requires_exactly_one_request() {
    let missing = run(["slice", "gate"].map(Into::into)).unwrap_err();
    let extra = run(["slice", "gate", "request.json", "extra.json"].map(Into::into)).unwrap_err();

    assert_eq!(missing, slice_gate_usage());
    assert_eq!(extra, slice_gate_usage());
}

// gate preparation은 compact source request와 별도의 immutable gate output 경로를
// 정확히 하나씩 요구하여 누락·덮어쓰기·추가 입력을 명령 경계에서 거부한다.
#[test]
fn slice_gate_prepare_requires_request_and_output() {
    let missing_both = run(["slice", "gate", "prepare"].map(Into::into)).unwrap_err();
    let missing_output =
        run(["slice", "gate", "prepare", "prepare.json"].map(Into::into)).unwrap_err();
    let extra = run([
        "slice",
        "gate",
        "prepare",
        "prepare.json",
        "gate.json",
        "extra.json",
    ]
    .map(Into::into))
    .unwrap_err();

    assert_eq!(missing_both, slice_gate_usage());
    assert_eq!(missing_output, slice_gate_usage());
    assert_eq!(extra, slice_gate_usage());
}

// Slice close는 plan 또는 apply와 정확히 하나의 대상을 요구하여, 누락된
// 정리 대상이나 조용히 무시되는 추가 인자가 파괴적 단계로 넘어가지 않는다.
#[test]
fn slice_close_rejects_incomplete_or_extra_arguments() {
    for arguments in [
        vec!["slice", "close"],
        vec!["slice", "close", "plan"],
        vec!["slice", "close", "apply"],
        vec!["slice", "close", "plan", "sample", "plan.json", "extra"],
        vec!["slice", "close", "apply", "plan.json", "extra"],
        vec!["slice", "close", "prepare"],
        vec!["slice", "close", "prepare", "request.json", "extra"],
        vec!["slice", "close", "unknown", "sample"],
    ] {
        assert_eq!(
            run(arguments.into_iter().map(Into::into)).unwrap_err(),
            slice_close_usage()
        );
    }
}

// accepted commit과 prepare는 각자의 exact 입력 개수만 받아, 누락되거나 추가된
// 경로로 다른 게이트 또는 메시지를 소비하지 않는다.
#[test]
fn slice_commit_requires_exactly_one_prepared_message() {
    let missing = run(["slice", "commit"].map(Into::into)).unwrap_err();
    let extra = run(["slice", "commit", "message", "extra"].map(Into::into)).unwrap_err();
    let prepare_missing =
        run(["slice", "commit", "prepare", "gate", "source"].map(Into::into)).unwrap_err();
    let prepare_extra = run([
        "slice", "commit", "prepare", "gate", "source", "out", "extra",
    ]
    .map(Into::into))
    .unwrap_err();

    assert_eq!(missing, slice_commit_usage());
    assert_eq!(extra, slice_commit_usage());
    assert_eq!(prepare_missing, slice_commit_usage());
    assert_eq!(prepare_extra, slice_commit_usage());
}
