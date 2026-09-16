use std::{cell::Cell, ffi::OsString, iter};

use super::{general_usage, run};

// 인자 없이 실행했을 때 서로 다른 입력 계약을 한 문장으로 섞지 않고,
// 인자 없는 검사와 커밋 입력 검사를 각각 실행 가능한 형태로 안내한다.
#[test]
fn general_usage_separates_argument_free_and_impact_checks() {
    let error = run(Vec::<OsString>::new()).unwrap_err();

    assert_eq!(
        error,
        "usage:\n\
             cargo xtask slice create <slice-contract.json>\n\
             cargo xtask slice create-activation <request.json>\n\
             cargo xtask slice review-packet [--check-readiness|--preflight] <request.json>\n\
             cargo xtask slice review-prepare <request.json>\n\
             cargo xtask slice review-delta <request.json>\n\
             cargo xtask slice review-egress <request.json>\n\
             cargo xtask slice review-target-admission <request.json>\n\
             cargo xtask slice review-deliver <request.json|finalize FINALIZE.json>\n\
             cargo xtask slice review-continuation-preflight <request.json>\n\
             cargo xtask slice review-result-correction-preflight <request.json>\n\
             cargo xtask slice cost-report <request.json> <output.json>\n\
             cargo xtask slice gate <request.json>\n\
             cargo xtask slice gate prepare <prepare.json> <gate.json>\n\
             cargo xtask slice close <prepare REQUEST.json|plan SLICE [PLAN.json]|apply PLAN.json>\n\
             cargo xtask slice commit <commit-message-file|prepare GATE.json MESSAGE-SOURCE MESSAGE-OUT>\n\
             cargo xtask slice accept <request.json|prepare PREPARE.json>\n\
             cargo xtask slice status <slice>\n\
             cargo xtask docs accept-translation <relative-page.md>\n\
             cargo xtask slice-contract bind <slice-contract.json>\n\
             cargo xtask check test-explanations\n\
             cargo xtask check methexis-check-for-stage\n\
             cargo xtask check slice-scope [slice-contract.json]\n\
             cargo xtask check slice-parallel <left.json> <right.json>\n\
             cargo xtask check wave-assembly <boundary.json> <component.json>...\n\
             cargo xtask check review-coverage-operation \
             <commit-message-file> [source] [commit]\n\
             cargo xtask check <change-preflight|commit-preflight|developer-docs-impact|slice-review-impact> \
             <commit-message-file> [changed-paths-file] [branch]"
    );
}

// 최상위 명령 분배는 첫 번째 명령이 없거나 알 수 없는 경우에도 기존 구현처럼 비교할 두 값을
// 모두 미리 읽어야 하므로, 호출 횟수를 관찰할 수 있는 반복자에서도 두 번 읽는 계약을 유지한다.
#[test]
fn general_dispatch_prefetches_command_and_scope() {
    let empty_calls = Cell::new(0);
    let empty = iter::from_fn(|| {
        empty_calls.set(empty_calls.get() + 1);
        None::<OsString>
    });
    assert!(run(empty).is_err());
    assert_eq!(empty_calls.get(), 2);

    let unknown_calls = Cell::new(0);
    let mut values = ["unknown", "scope"].into_iter().map(OsString::from);
    let unknown = iter::from_fn(|| {
        unknown_calls.set(unknown_calls.get() + 1);
        values.next()
    });
    assert!(run(unknown).is_err());
    assert_eq!(unknown_calls.get(), 2);
}

mod lint;
