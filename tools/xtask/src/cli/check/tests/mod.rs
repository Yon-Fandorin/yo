use std::ffi::OsString;

use super::usage;
use crate::cli::run;

// test-explanations 뒤의 불필요한 인자는 조용히 무시하지 않고 해당 명령의
// 정확한 사용법을 돌려줘 호출자가 잘못 구성한 훅을 바로 고칠 수 있게 한다.
#[test]
fn test_explanations_rejects_extra_arguments_with_specific_usage() {
    let error = run(["check", "test-explanations", "unexpected"].map(Into::into)).unwrap_err();

    assert_eq!(error, "usage: cargo xtask check test-explanations");
}

// prepare-commit-msg 경계는 Git이 전달하는 메시지 파일과 선택적 source/commit만
// 받아, 누락되거나 추가된 hook 인자를 다른 커밋 동작으로 오인하지 않는다.
#[test]
fn review_coverage_operation_requires_the_prepare_commit_message_shape() {
    let expected = super::usage("review-coverage-operation");
    let missing = run(["check", "review-coverage-operation"].map(Into::into)).unwrap_err();
    let extra = run([
        "check",
        "review-coverage-operation",
        "message",
        "commit",
        "0123456789abcdef0123456789abcdef01234567",
        "extra",
    ]
    .map(Into::into))
    .unwrap_err();

    assert_eq!(missing, expected);
    assert_eq!(extra, expected);
}
