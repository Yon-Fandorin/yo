use super::check;
use crate::impact::ImpactInput;

struct Case {
    name: &'static str,
    message: &'static str,
    paths: &'static [&'static str],
    branch: &'static str,
    passes: bool,
}

// 문서 전용 변경, 필수 trailer의 누락·중복·값 형식, Slice 유예를 한 표에서 검증해
// 기존 hook이 허용하거나 차단하던 Developer Docs 영향 계약을 그대로 보존한다.
#[test]
fn preserves_developer_docs_impact_contract() {
    let cases = [
        Case {
            name: "docs-only changes need no impact trailer",
            message: "docs: explain the runtime",
            paths: &["docs/src/architecture/runtime-flow.md"],
            branch: "develop",
            passes: true,
        },
        Case {
            name: "code changes cannot omit the trailer",
            message: "refactor(core): move runtime ownership",
            paths: &["crates/yo-core/src/runtime/mod.rs"],
            branch: "develop",
            passes: false,
        },
        Case {
            name: "shared library changes cannot omit the trailer",
            message: "refactor(yaml): revise shared parser",
            paths: &["shared/yo-yaml/src/lib.rs"],
            branch: "develop",
            passes: false,
        },
        Case {
            name: "cargo alias changes cannot omit the trailer",
            message: "build: revise repository command alias",
            paths: &[".cargo/config.toml"],
            branch: "develop",
            passes: false,
        },
        Case {
            name: "updated requires a Developer Docs change",
            message: "refactor(core): move runtime ownership\n\nDeveloper-Docs-Impact: updated",
            paths: &["crates/yo-core/src/runtime/mod.rs"],
            branch: "develop",
            passes: false,
        },
        Case {
            name: "updated accepts a staged Developer Docs change",
            message: "refactor(core): move runtime ownership\n\nDeveloper-Docs-Impact: updated",
            paths: &[
                "crates/yo-core/src/runtime/mod.rs",
                "docs/src/architecture/runtime-flow.md",
            ],
            branch: "develop",
            passes: true,
        },
        Case {
            name: "none requires a concrete reason",
            message: "fix(tui): correct a typo\n\nDeveloper-Docs-Impact: none",
            paths: &["crates/yo-tui/src/lib.rs"],
            branch: "develop",
            passes: false,
        },
        Case {
            name: "none accepts a concrete reason",
            message: "fix(tui): correct a typo\n\nDeveloper-Docs-Impact: none - exported responsibilities and runtime flow are unchanged",
            paths: &["crates/yo-tui/src/lib.rs"],
            branch: "develop",
            passes: true,
        },
        Case {
            name: "multiple trailers are ambiguous",
            message: "fix(tui): correct a typo\n\nDeveloper-Docs-Impact: updated\nDeveloper-Docs-Impact: none - no flow change",
            paths: &[
                "crates/yo-tui/src/lib.rs",
                "docs/src/architecture/overview.md",
            ],
            branch: "develop",
            passes: false,
        },
        Case {
            name: "working Slice commits defer the decision",
            message: "refactor(core): move runtime ownership",
            paths: &["crates/yo-core/src/runtime/mod.rs"],
            branch: "slice/direct/runtime-ownership",
            passes: true,
        },
    ];

    for case in cases {
        let result = check(&input(case.message, case.paths, case.branch));
        assert_eq!(result.is_ok(), case.passes, "{}: {result:?}", case.name);
    }
}

fn input(message: &str, paths: &[&str], branch: &str) -> ImpactInput {
    ImpactInput {
        message: message.to_owned(),
        changed_paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        branch: branch.to_owned(),
        merge_head: None,
        repository: ".".into(),
        inherit_git_environment: true,
    }
}
