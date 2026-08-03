use super::{
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceScope,
    search_candidates,
};

fn candidate(identity: &str, name: &str, scope: SkillReferenceScope) -> SkillReferenceCandidate {
    SkillReferenceCandidate::new(
        SkillReference::new(
            identity,
            "local:host",
            identity,
            name,
            scope,
            1,
            "metadata:1",
        ),
        name,
        format!("Use {name} workflows"),
        SkillAvailability::Enabled,
    )
}

// 같은 이름의 스킬이 여러 scope에 있어도 opaque identity가 다르면 별도 후보로 보존되어
// 사용자가 출처를 보고 정확한 항목을 선택할 수 있다.
#[test]
fn duplicate_names_from_different_scopes_remain_separate_candidates() {
    let inventory = vec![
        candidate("repo:path", "review", SkillReferenceScope::Workspace),
        candidate("user:path", "review", SkillReferenceScope::User),
    ];

    let results = search_candidates(&inventory, "review");

    assert_eq!(results.len(), 2);
    assert_ne!(
        results[0].reference().identity(),
        results[1].reference().identity()
    );
}

// 이름이 정확히 일치하는 후보는 설명에만 검색어가 있는 후보보다 먼저 나와,
// 작은 화면에서도 가장 직접적인 결과가 안정적으로 보인다.
#[test]
fn exact_name_match_precedes_description_only_match() {
    let inventory = vec![
        candidate("description", "helper", SkillReferenceScope::User),
        candidate("exact", "review", SkillReferenceScope::Workspace),
    ];

    let results = search_candidates(&inventory, "review");

    assert_eq!(results[0].reference().identity(), "exact");
}
