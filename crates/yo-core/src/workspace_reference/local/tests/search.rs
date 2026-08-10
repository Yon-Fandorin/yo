use super::super::{rank, search};
use crate::{WorkspaceReference, WorkspaceReferenceCandidate, WorkspaceReferenceKind};

fn candidate(path: &str) -> WorkspaceReferenceCandidate {
    WorkspaceReferenceCandidate::new(
        WorkspaceReference::new(
            path,
            "environment",
            "workspace",
            "root",
            path,
            WorkspaceReferenceKind::File,
        )
        .unwrap(),
    )
}

// basename의 정확 일치와 접두 일치는 단순 경로 포함보다 먼저 나오며 동률은 경로순으로 고정된다.
#[test]
fn ranking_prioritizes_familiar_path_matches_deterministically() {
    let entries = vec![
        candidate("src/main.rs"),
        candidate("notes/main-guide.md"),
        candidate("examples/domain.rs"),
    ];
    let results = search(&entries, "main");
    let paths = results
        .iter()
        .map(|entry| entry.reference().relative_path())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec!["src/main.rs", "notes/main-guide.md", "examples/domain.rs"]
    );
    assert_eq!(results[0].reference().relative_path(), "src/main.rs");
    assert_eq!(
        results[1].reference().relative_path(),
        "notes/main-guide.md"
    );
    assert!(rank("src/main.rs", "main.rs", "src/main.rs") < rank("src/main.rs", "main.rs", "main"));
    assert!(rank("src/main.rs", "main.rs", "missing").is_none());
}

// 디렉터리 표시용 `/`는 검색 점수에 섞지 않아 basename 정확 일치를 그대로 보존한다.
#[test]
fn directory_decoration_does_not_weaken_an_exact_basename_match() {
    assert_eq!(
        rank("src/components", "components", "components")
            .unwrap()
            .0,
        0
    );
}

// 조합형 대문자 query가 정규화되고 결과 path가 match tier 순서로 정렬되는지 확인한다.
#[test]
fn search_normalizes_queries_and_orders_match_tiers() {
    let entries = vec![
        candidate("src/Caf\u{e9}"),
        candidate("caf\u{e9}-notes.md"),
        candidate("docs/caf\u{e9}-guide.md"),
        candidate("docs/caf\u{e9}/readme.md"),
        candidate("notes/decaf\u{e9}.md"),
        candidate("x/c-a-f-\u{e9}.md"),
    ];
    let results = search(&entries, "CAFE\u{301}/");
    let paths = results
        .iter()
        .map(|entry| entry.reference().relative_path())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            "src/Café",
            "café-notes.md",
            "docs/café-guide.md",
            "docs/café/readme.md",
            "notes/decafé.md",
            "x/c-a-f-é.md",
        ]
    );
}
