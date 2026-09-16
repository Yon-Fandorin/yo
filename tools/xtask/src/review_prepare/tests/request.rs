use serde_json::json;

use super::{
    super::{
        model::Target,
        request::{prepared_review_questions, validate_and_normalize},
    },
    sample_request,
};
use crate::review_result;

// 입력 정규화는 중복 authority와 지원되지 않는 host를 초기에 거부하여 ContextBuild나
// packet publication을 시작하기 전에 사람이 고칠 수 있는 작은 오류로 남깁니다.
#[test]
fn input_validation_rejects_duplicates_and_unknown_hosts() {
    let mut duplicate = sample_request(json!({"kind": "delegated_host", "host": "codex"}));
    duplicate.repository_authority_paths = vec!["CONTRIBUTING.md".into(), "CONTRIBUTING.md".into()];
    assert!(
        validate_and_normalize(&mut duplicate)
            .unwrap_err()
            .contains("duplicate value")
    );

    let mut unknown = sample_request(json!({"kind": "delegated_host", "host": "other"}));
    assert!(
        validate_and_normalize(&mut unknown)
            .unwrap_err()
            .contains("must be `codex` or `grok`")
    );
    assert!(matches!(unknown.target, Target::DelegatedHost { .. }));
}

// alpha2 통합 경로만 terminal structured-result 지시를 정확히 한 번 추가하고,
// 이미 발행된 alpha1 준비 의미와 호출자가 작성한 질문은 그대로 보존합니다.
#[test]
fn alpha2_adds_structured_result_instruction_without_reinterpreting_alpha1() {
    let legacy = sample_request(json!({"kind": "delegated_host", "host": "codex"}));
    assert_eq!(prepared_review_questions(&legacy), legacy.review_questions);

    let mut structured = legacy;
    structured.schema = "yo.slice-review-prepare-request/v1alpha2".to_owned();
    let questions = prepared_review_questions(&structured);
    assert_eq!(questions.len(), structured.review_questions.len() + 1);
    assert!(
        questions
            .last()
            .unwrap()
            .contains("yo.slice-review-result/v1alpha1")
    );
    validate_and_normalize(&mut structured).unwrap();
}

// alpha4는 기존 alpha1 policy를 계속 요구해 이미 발행된 요청의 라우팅 의미가 새
// 세분화 정책으로 바뀌지 않도록 동결합니다.
#[test]
fn alpha4_requires_derived_authority_policy() {
    let mut derived = sample_request(json!({"kind": "delegated_host", "host": "grok"}));
    derived.schema = "yo.slice-review-prepare-request/v1alpha4".to_owned();
    derived.repository_authority_paths.clear();
    derived.repository_authority_policy = Some("changed-workflow-authority/v1alpha1".to_owned());
    validate_and_normalize(&mut derived).unwrap();

    derived.repository_authority_paths = vec!["CONTRIBUTING.md".to_owned()];
    assert!(
        validate_and_normalize(&mut derived)
            .unwrap_err()
            .contains("requires the caller list to be empty")
    );
}

// alpha5는 caller 목록을 금지하고 alpha2 policy만 받아, 세분화된 owner 선택이 임의
// authority 누락이나 alpha4 의미 재해석으로 바뀌지 않게 합니다.
#[test]
fn alpha5_requires_precise_derived_authority_policy() {
    let mut derived = sample_request(json!({"kind": "delegated_host", "host": "grok"}));
    derived.schema = "yo.slice-review-prepare-request/v1alpha5".to_owned();
    derived.repository_authority_paths.clear();
    derived.repository_authority_policy = Some("changed-workflow-authority/v1alpha2".to_owned());
    validate_and_normalize(&mut derived).unwrap();

    derived.repository_authority_policy = Some("changed-workflow-authority/v1alpha1".to_owned());
    assert!(
        validate_and_normalize(&mut derived)
            .unwrap_err()
            .contains("changed-workflow-authority/v1alpha2")
    );
}
