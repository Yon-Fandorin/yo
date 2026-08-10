use super::super::{PathRule, overlaps};

// 디렉터리 규칙은 그 디렉터리 자체와 하위 파일을 허용하지만 이름 접두사가
// 우연히 같은 이웃 디렉터리까지 허용하지 않아 Slice 경계를 넘지 않는다.
#[test]
fn tree_rule_matches_only_its_repository_subtree() {
    let rule = PathRule::parse("crates/yo-tui/src/**").unwrap();

    assert!(rule.matches("crates/yo-tui/src"));
    assert!(rule.matches("crates/yo-tui/src/render/mod.rs"));
    assert!(!rule.matches("crates/yo-tui/src-old/render.rs"));
}

// 한 Slice의 디렉터리 lease가 다른 Slice의 파일 lease를 포함하면 실제
// 변경 전이라도 병렬 충돌로 찾아 공용 파일의 이중 writer를 막는다.
#[test]
fn overlap_detects_a_file_inside_another_slice_tree() {
    let left = [PathRule::parse("crates/yo-tui/src/**").unwrap()];
    let right = [PathRule::parse("crates/yo-tui/src/lib.rs").unwrap()];

    assert_eq!(overlaps(&left, &right).len(), 1);
}

// 서로 다른 crate의 하위 트리는 같은 파일을 쓸 수 없으므로 병렬 실행
// 가능 대상으로 유지하고 불필요하게 저장소 전체를 직렬화하지 않는다.
#[test]
fn disjoint_crate_trees_can_run_in_parallel() {
    let left = [PathRule::parse("crates/yo-tui/src/**").unwrap()];
    let right = [PathRule::parse("crates/yo-core/src/journal/**").unwrap()];

    assert!(overlaps(&left, &right).is_empty());
}
