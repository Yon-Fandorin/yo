use std::path::PathBuf;

use super::{check_from, check_in, check_paths};
use crate::test_support::TestRepository;

// 직전 줄의 일반·doc line-comment는 기존 검사와 같이 설명으로 인정하지만, 빈 줄이나
// block comment는 인정하지 않아 #[test]와 설명 사이의 즉시 인접 계약을 보존한다.
#[test]
fn accepts_only_an_immediately_preceding_line_comment() {
    let repository = TestRepository::new("test-explanation-comments");
    repository.write(
        "tools/example/src/lib.rs",
        "// 일반 설명\n#[test]\nfn ordinary() {}\n\
         /// 문서화 설명\n    #[test]   \nfn documented() {}\n\
         // 떨어진 설명\n\n#[test]\nfn separated() {}\n\
         /* block 설명 */\n#[test]\nfn blocked() {}\n",
    );

    let result = check_paths(
        &repository.path,
        &[PathBuf::from("tools/example/src/lib.rs")],
    )
    .unwrap_err();

    assert!(result.contains("tools/example/src/lib.rs:9:"));
    assert!(result.contains("tools/example/src/lib.rs:12:"));
    assert_eq!(result.lines().count(), 2);
}

// 여러 파일의 누락을 경로와 실제 1-based 행 번호 순서로 모두 보고해, 첫 오류 뒤의
// 테스트 설명 누락도 한 번의 검사에서 함께 고칠 수 있게 한다.
#[test]
fn reports_every_missing_explanation_in_stable_path_order() {
    let repository = TestRepository::new("test-explanation-order");
    repository.write("tools/z.rs", "#[test]\nfn z() {}\n");
    repository.write("crates/a.rs", "\n#[test]\nfn a() {}\n");

    let result = check_paths(
        &repository.path,
        &[PathBuf::from("crates/a.rs"), PathBuf::from("tools/z.rs")],
    )
    .unwrap_err();

    assert_eq!(
        result.lines().collect::<Vec<_>>(),
        [
            "crates/a.rs:2: #[test] requires an explanatory line-comment immediately above it; review verifies Korean readability",
            "tools/z.rs:1: #[test] requires an explanatory line-comment immediately above it; review verifies Korean readability",
        ]
    );
}

// 실제 Git 파일 선택은 tracked 파일과 ignore되지 않은 새 파일을 검사하되 .gitignore
// 대상은 제외해, 기존 `rg --files`가 pre-commit에서 보던 작업 사본 범위를 유지한다.
#[test]
fn scans_tracked_and_untracked_rust_sources_but_not_ignored_files() {
    let repository = TestRepository::new("test-explanation-file-selection");
    repository.write(".gitignore", "tools/ignored.rs\n");
    repository.write(
        "crates/tracked.rs",
        "// tracked 설명\n#[test]\nfn tracked() {}\n",
    );
    repository.git(["add", ".gitignore", "crates/tracked.rs"]);
    repository.write(
        "tools/untracked.rs",
        "// untracked 설명\n#[test]\nfn untracked() {}\n",
    );
    repository.write("tools/ignored.rs", "#[test]\nfn ignored() {}\n");

    assert!(check_in(&repository.path, false).is_ok());

    repository.write("tools/untracked.rs", "#[test]\nfn untracked() {}\n");
    let result = check_in(&repository.path, false).unwrap_err();
    assert!(result.starts_with("tools/untracked.rs:1:"));
    assert!(!result.contains("ignored.rs"));
}

// 작업 사본에서 삭제된 tracked 파일은 기존 `rg --files`와 같이 검사 대상에서 빠져,
// 파일 삭제나 이름 변경 자체가 설명 검사 오류로 바뀌지 않는다.
#[test]
fn skips_tracked_sources_deleted_from_the_working_tree() {
    let repository = TestRepository::new("test-explanation-deleted-source");
    let deleted = repository.write("crates/deleted.rs", "#[test]\nfn deleted() {}\n");
    repository.git(["add", "crates/deleted.rs"]);
    std::fs::remove_file(deleted).unwrap();

    assert!(check_in(&repository.path, false).is_ok());
}

// Git이 따옴표로 이스케이프할 수 있는 한글 파일명도 NUL 구분 경로로 그대로 읽어,
// 파일명 표현 방식 때문에 설명 누락이 검사에서 빠지지 않는다.
#[test]
fn scans_non_ascii_rust_paths() {
    let repository = TestRepository::new("test-explanation-non-ascii-path");
    repository.write("tools/한글.rs", "#[test]\nfn untranslated() {}\n");

    let result = check_in(&repository.path, false).unwrap_err();

    assert!(result.starts_with("tools/한글.rs:1:"));
}

// 기존 검사처럼 test 속성 뒤에 공백과 주석이 붙거나 CRLF 줄바꿈을 쓰더라도
// `#[test]`로 인식해, 바로 위 설명이 없으면 누락으로 보고한다.
#[test]
fn recognizes_trailing_text_and_crlf_after_test_attribute() {
    let repository = TestRepository::new("test-explanation-attribute-boundary");
    repository.write(
        "tools/example.rs",
        "#[test] // trailing comment\r\nfn trailing() {}\r\n\
         // 설명\r\n#[test]\t// trailing comment\r\nfn explained() {}\r\n\
         #[test]suffix\r\nfn not_an_attribute() {}\r\n",
    );

    let result = check_paths(&repository.path, &[PathBuf::from("tools/example.rs")]).unwrap_err();

    assert_eq!(result.lines().count(), 1);
    assert!(result.starts_with("tools/example.rs:1:"));
}

// 저장소 하위 디렉터리에서 실행해도 Git 최상위를 기준으로 crates와 tools를 찾아,
// 빈 범위를 검사한 뒤 성공하는 대신 실제 설명 누락을 보고한다.
#[test]
fn resolves_the_repository_root_from_a_nested_directory() {
    let repository = TestRepository::new("test-explanation-nested-directory");
    repository.write("crates/missing.rs", "#[test]\nfn missing() {}\n");
    let nested = repository.write("tools/example/.keep", "");

    let result = check_from(nested.parent().unwrap(), false).unwrap_err();

    assert!(result.starts_with("crates/missing.rs:1:"));
}
