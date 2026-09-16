use std::ffi::OsString;

use super::docs_accept_translation_usage;
use crate::cli::run;

// 번역 승인 명령은 검토할 한 페이지를 반드시 요구하고 추가 인자를
// 무시하지 않아, 호출자가 의도치 않게 여러 페이지를 승인하지 못하게 한다.
#[test]
fn docs_accept_translation_requires_exactly_one_page() {
    let missing = run(["docs", "accept-translation"].map(Into::into)).unwrap_err();
    let extra =
        run(["docs", "accept-translation", "README.md", "extra.md"].map(Into::into)).unwrap_err();

    assert_eq!(missing, docs_accept_translation_usage());
    assert_eq!(extra, docs_accept_translation_usage());
}
