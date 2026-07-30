use super::check;
use crate::impact::ImpactInput;

struct Case {
    name: &'static str,
    message: &'static str,
    path: &'static str,
    branch: &'static str,
    passes: bool,
}

// 완료된 lens 문법, reviewer ID, 중복과 미완료 표현을 함께 검사해 사람이 수행하지 않은
// 검수가 accepted commit의 증거처럼 통과하지 못하게 한다.
#[test]
fn accepts_only_completed_unambiguous_review_evidence() {
    let cases = [
        Case {
            name: "ordinary docs can record none",
            message: "docs: clarify\n\nSlice-Review: none - wording only",
            path: "docs/src/architecture/overview.md",
            branch: "develop",
            passes: true,
        },
        Case {
            name: "production code accepts both required lenses",
            message: "fix: restore\n\nSlice-Review: fresh-context - completed - reviewer/contract - clear\nSlice-Review: code-quality - completed - reviewer/quality - resolved",
            path: "crates/yo-tui/src/lib.rs",
            branch: "develop",
            passes: true,
        },
        Case {
            name: "human review is valid",
            message: "docs: workflow\n\nSlice-Review: fresh-context - completed - human/yon - clear",
            path: "CONTRIBUTING.md",
            branch: "develop",
            passes: true,
        },
        Case {
            name: "missing disposition fails",
            message: "docs: clarify",
            path: "docs/src/architecture/overview.md",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "none cannot replace code reviews",
            message: "fix: restore\n\nSlice-Review: none - tests pass",
            path: "crates/yo-tui/src/lib.rs",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "quota exhaustion is not completed",
            message: "docs: workflow\n\nSlice-Review: fresh-context - Kimi quota exhausted; review not performed",
            path: "CONTRIBUTING.md",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "pending fallback is not completed",
            message: "docs: workflow\n\nSlice-Review: fresh-context - Kimi unavailable; pending review by Codex",
            path: "CONTRIBUTING.md",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "unknown outcome is invalid",
            message: "docs: workflow\n\nSlice-Review: fresh-context - completed - codex/session - pending",
            path: "CONTRIBUTING.md",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "free-form suffix is invalid",
            message: "docs: workflow\n\nSlice-Review: fresh-context - completed - codex/session - clear - unfinished",
            path: "CONTRIBUTING.md",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "fresh-context does not replace quality",
            message: "fix: restore\n\nSlice-Review: fresh-context - completed - reviewer/contract - clear",
            path: "crates/yo-tui/src/lib.rs",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "quality does not replace fresh-context",
            message: "fix: restore\n\nSlice-Review: code-quality - completed - reviewer/quality - clear",
            path: "crates/yo-tui/src/lib.rs",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "none cannot accompany completed review",
            message: "docs: workflow\n\nSlice-Review: fresh-context - completed - reviewer/contract - clear\nSlice-Review: none - no review needed",
            path: "CONTRIBUTING.md",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "duplicate fresh-context is ambiguous",
            message: "fix: restore\n\nSlice-Review: fresh-context - completed - reviewer/one - clear\nSlice-Review: fresh-context - completed - reviewer/two - clear\nSlice-Review: code-quality - completed - reviewer/quality - clear",
            path: "crates/yo-tui/src/lib.rs",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "review-looking prose is not a trailer",
            message: "docs: clarify\n\nThe example Slice-Review: none - wording only is not evidence.",
            path: "docs/src/architecture/overview.md",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "working Slice defers review",
            message: "fix: restore",
            path: "crates/yo-tui/src/lib.rs",
            branch: "slice/direct/terminal-restoration",
            passes: true,
        },
    ];

    assert_cases(&cases);
}

// 경로별 최소 lens와 Wave integration 요구를 표로 검증해 도구 설정, 실행 코드,
// Developer Docs theme, 공개 문서와 SOT가 기존보다 약한 검수로 통과하지 못하게 한다.
#[test]
fn preserves_path_and_wave_review_requirements() {
    let cases = [
        Case {
            name: "tool scripts require quality",
            message: "test: tool\n\nSlice-Review: fresh-context - completed - reviewer/contract - clear",
            path: "tools/validation/example.sh",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "tool manifests require fresh context",
            message: "build: tool\n\nSlice-Review: fresh-context - completed - reviewer/contract - clear",
            path: "tools/example/Cargo.toml",
            branch: "develop",
            passes: true,
        },
        Case {
            name: "cargo aliases require fresh context",
            message: "build: alias\n\nSlice-Review: none - configuration only",
            path: ".cargo/config.toml",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "Developer Docs theme requires quality",
            message: "docs: theme\n\nSlice-Review: none - docs only",
            path: "docs/theme/language-switch.js",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "public orientation requires fresh context",
            message: "docs: scope\n\nSlice-Review: none - prose only",
            path: "README.md",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "SOT authority requires fresh context",
            message: "docs: sot\n\nSlice-Review: none - prose only",
            path: "methexis/knowledge/unit.md",
            branch: "develop",
            passes: false,
        },
        Case {
            name: "Wave code requires integration",
            message: "feat: core\n\nSlice-Review: fresh-context - completed - reviewer/contract - clear\nSlice-Review: code-quality - completed - reviewer/quality - clear",
            path: "crates/yo-core/src/lib.rs",
            branch: "wave/w1-runtime",
            passes: false,
        },
        Case {
            name: "Wave accepts all three lenses",
            message: "feat: core\n\nSlice-Review: fresh-context - completed - reviewer/contract - clear\nSlice-Review: code-quality - completed - reviewer/quality - clear\nSlice-Review: integration - completed - reviewer/wave - clear",
            path: "crates/yo-core/src/lib.rs",
            branch: "wave/w1-runtime",
            passes: true,
        },
        Case {
            name: "Wave docs still require integration",
            message: "docs: clarify\n\nSlice-Review: none - wording only",
            path: "docs/src/architecture/overview.md",
            branch: "wave/w1-runtime",
            passes: false,
        },
    ];

    assert_cases(&cases);
}

fn assert_cases(cases: &[Case]) {
    for case in cases {
        let result = check(&ImpactInput {
            message: case.message.to_owned(),
            changed_paths: vec![case.path.to_owned()],
            branch: case.branch.to_owned(),
            merge_head: None,
            repository: ".".into(),
            inherit_git_environment: true,
        });
        assert_eq!(result.is_ok(), case.passes, "{}: {result:?}", case.name);
    }
}
