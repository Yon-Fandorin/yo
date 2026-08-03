use super::{check, combine};
use crate::{impact::ImpactInput, test_support::TestRepository};

// 두 impact 검사가 모두 성공하면 preflight도 추가 출력 없이 성공해 준비된 메시지를
// 그대로 실제 commit에 사용할 수 있는지 확인한다.
#[test]
fn succeeds_only_when_both_impact_checks_succeed() {
    assert_eq!(combine(Ok(()), Ok(())), Ok(()));
}

// 두 검사가 함께 실패하면 첫 오류에서 멈추지 않고 각각의 소유 영역과 진단을 한 번에
// 보여줘 비싼 commit hook을 반복 실행하기 전에 메시지를 모두 고칠 수 있게 한다.
#[test]
fn reports_both_impact_failures_in_one_bounded_diagnostic() {
    let error = combine(
        Err("missing review trailer".to_owned()),
        Err("missing docs trailer".to_owned()),
    )
    .unwrap_err();

    assert_eq!(
        error,
        "commit preflight failed:\n\n\
         Slice review impact:\nmissing review trailer\n\n\
         Developer Docs impact:\nmissing docs trailer"
    );
}

// 한 검사만 실패한 경우 성공한 영역을 오류처럼 출력하지 않고 실패한 소유 영역만
// 표시해 진단의 원인을 과장하지 않는지 확인한다.
#[test]
fn reports_only_the_failed_impact_boundary() {
    let error = combine(Ok(()), Err("invalid docs trailer".to_owned())).unwrap_err();

    assert_eq!(
        error,
        "commit preflight failed:\n\nDeveloper Docs impact:\ninvalid docs trailer"
    );
}

// clean-index amend 상황에서도 HEAD의 실제 도구 변경 경로를 불러와 두 validator를 모두
// 실행하고, 준비 메시지에 두 trailer가 없으면 두 진단을 한 번에 반환하는지 확인한다.
#[test]
fn clean_index_head_fallback_runs_both_real_validators() {
    let repository = TestRepository::new("preflight-clean-amend-invalid");
    repository.write("tools/example/check.sh", "reviewed\n");
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "test: seed tool change"]);
    let message = repository.write("message", "test: rewrite message\n");
    let input = ImpactInput::load_from(
        &repository.path,
        message,
        None,
        Some("develop".to_owned()),
        true,
    )
    .unwrap();

    let error = check(&input).unwrap_err();

    assert!(error.contains("Slice review impact:"));
    assert!(error.contains("Developer Docs impact:"));
    assert!(error.contains("changed: tools/example/check.sh"));
}

// 같은 clean-index HEAD 변경에 필요한 두 trailer를 모두 기록하면 실제 validator를 거친
// combined preflight가 성공해 유효한 amend 메시지를 거부하지 않는지 확인한다.
#[test]
fn clean_index_head_fallback_accepts_a_complete_prepared_message() {
    let repository = TestRepository::new("preflight-clean-amend-valid");
    repository.write("tools/example/check.sh", "reviewed\n");
    repository.git(["add", "."]);
    repository.git(["commit", "--quiet", "-m", "test: seed tool change"]);
    let message = repository.write(
        "message",
        "test: rewrite message\n\n\
         Slice-Review: fresh-context - completed - codex/test - clear\n\
         Slice-Review: code-quality - completed - codex/test - clear\n\
         Developer-Docs-Impact: none - runtime responsibilities remain unchanged\n",
    );
    let input = ImpactInput::load_from(
        &repository.path,
        message,
        None,
        Some("develop".to_owned()),
        true,
    )
    .unwrap();

    assert_eq!(check(&input), Ok(()));
}
