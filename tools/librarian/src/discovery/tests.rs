use super::discover;
use crate::{
    catalog::{Catalog, Unit},
    wire::{Anchor, CandidateReason, DiscoveryRequest, REQUEST_SCHEMA},
};

fn catalog() -> Catalog {
    let units = ["test.alpha", "test.beta"]
        .into_iter()
        .map(|id| {
            (
                id.to_owned(),
                Unit {
                    id: id.to_owned(),
                    revision: format!("revision-{id}"),
                    owner: "test".to_owned(),
                    sources: Vec::new(),
                    path: format!("methexis/knowledge/{id}.md"),
                    title: "Shared title".to_owned(),
                    body: "shared body".to_owned(),
                    projection: None,
                    relations: Default::default(),
                },
            )
        })
        .collect();
    Catalog {
        units,
        hash: "catalog-hash".to_owned(),
    }
}

// 중복 anchor는 점수에 흡수시키지 않고 duplicate_anchor 오류로 거부한다.
#[test]
fn duplicate_anchors_are_rejected_instead_of_hiding_score() {
    let request = DiscoveryRequest {
        schema: REQUEST_SCHEMA.to_owned(),
        query: None,
        anchors: vec![
            Anchor::KnowledgeId {
                value: "test.alpha".to_owned(),
            },
            Anchor::KnowledgeId {
                value: "test.alpha".to_owned(),
            },
        ],
    };

    let error = discover(request, &catalog())
        .expect_err("duplicate anchor must fail")
        .into_envelope();

    assert_eq!(error.error.code, "duplicate_anchor");
}

// 앞뒤 공백만 다른 같은 anchor도 중복으로 판정해 duplicate_anchor로 거부한다.
#[test]
fn whitespace_variants_of_the_same_anchor_are_duplicates() {
    let request = DiscoveryRequest {
        schema: REQUEST_SCHEMA.to_owned(),
        query: None,
        anchors: vec![
            Anchor::KnowledgeId {
                value: "test.alpha".to_owned(),
            },
            Anchor::KnowledgeId {
                value: " test.alpha ".to_owned(),
            },
        ],
    };

    let error = discover(request, &catalog()).expect_err("semantic duplicate must fail");

    assert_eq!(error.into_envelope().error.code, "duplicate_anchor");
}

// query가 KnowledgeId와 정확히 같으면 그 사실을 ExactQuery라는 첫 번째 점수 근거로 남긴다.
// 최종 score를 근거별 점수의 합으로 다시 계산할 수 있어야 한다.
#[test]
fn exact_id_query_has_distinct_and_reconstructible_evidence() {
    let request = DiscoveryRequest {
        schema: REQUEST_SCHEMA.to_owned(),
        query: Some("test.alpha".to_owned()),
        anchors: Vec::new(),
    };

    let result = discover(request, &catalog()).expect("discovery succeeds");
    let candidate = &result.candidates[0];
    let explained_score = candidate.reasons.iter().map(reason_score).sum::<u64>();

    assert_eq!(candidate.id, "test.alpha");
    assert!(matches!(
        candidate.reasons.first(),
        Some(CandidateReason::ExactQuery { field: "id", .. })
    ));
    assert_eq!(candidate.score, explained_score);
}

// 동점 candidate는 knowledge id 순으로 정렬된다.
#[test]
fn equal_scores_are_ordered_by_knowledge_id() {
    let request = DiscoveryRequest {
        schema: REQUEST_SCHEMA.to_owned(),
        query: Some("shared".to_owned()),
        anchors: Vec::new(),
    };

    let result = discover(request, &catalog()).expect("discovery succeeds");
    let ids = result
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(ids, ["test.alpha", "test.beta"]);
}

// 같은 검색어가 여러 관계 대상에 나타나는 후보에서도 최종 score를 숨은 규칙으로 만들지 않는다.
// 응답에 공개한 reason별 점수를 모두 더하면 최종 score를 그대로 재구성할 수 있어야 한다.
#[test]
fn repeated_term_across_relation_targets_has_one_explained_contribution() {
    let mut catalog = catalog();
    catalog
        .units
        .get_mut("test.alpha")
        .expect("unit")
        .relations
        .validated_by = vec![
        "shared.fixture-one".to_owned(),
        "shared.fixture-two".to_owned(),
    ];
    let request = DiscoveryRequest {
        schema: REQUEST_SCHEMA.to_owned(),
        query: Some("shared".to_owned()),
        anchors: Vec::new(),
    };

    let result = discover(request, &catalog).expect("discovery succeeds");
    let candidate = result
        .candidates
        .iter()
        .find(|candidate| candidate.id == "test.alpha")
        .expect("alpha candidate");

    assert_eq!(
        candidate.score,
        candidate.reasons.iter().map(reason_score).sum::<u64>()
    );
}

// 여러 anchor 중 일부만 지식을 찾더라도 성공한 결과를 버리거나 실패한 항목을 숨기지 않는다.
// 찾은 지식은 candidates에, 찾지 못한 anchor는 unresolved_anchors에 각각 남긴다.
#[test]
fn mixed_resolved_and_unresolved_anchors_remain_explicit() {
    let request = DiscoveryRequest {
        schema: REQUEST_SCHEMA.to_owned(),
        query: None,
        anchors: vec![
            Anchor::KnowledgeId {
                value: "test.alpha".to_owned(),
            },
            Anchor::Symbol {
                value: "missing::symbol".to_owned(),
            },
        ],
    };

    let result = discover(request, &catalog()).expect("discovery succeeds");

    assert_eq!(result.candidates[0].id, "test.alpha");
    assert_eq!(result.unresolved_anchors.len(), 1);
}

fn reason_score(reason: &CandidateReason) -> u64 {
    match reason {
        CandidateReason::Anchor { score, .. }
        | CandidateReason::ExactQuery { score, .. }
        | CandidateReason::QueryPhrase { score, .. }
        | CandidateReason::QueryToken { score, .. }
        | CandidateReason::Relation { score, .. } => *score,
    }
}
