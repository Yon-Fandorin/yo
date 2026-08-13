use super::super::{
    ApiDialect, EffectiveModelProfile, ModelProfileLayer, ModelProfileParameters,
    VersionedProfileId,
};

fn parameters(yaml: &str) -> ModelProfileParameters {
    serde_norway::from_str(yaml).unwrap()
}

fn id(value: &str) -> VersionedProfileId {
    VersionedProfileId::new(value).unwrap()
}

fn complete_layer() -> ModelProfileLayer {
    ModelProfileLayer::new(
        Some(ApiDialect::OpenAiResponses),
        Some(id("utf8-bytes/v1")),
        Some(100_000),
        Some(8_192),
        Some(parameters("{effort: medium}")),
        Some(parameters("{parallel_tool_calls: true}")),
        Some(id("local-tools/v1")),
        Some(id("semantic-terminal/v1")),
    )
}

// 상위 profile의 모든 필드를 먼저 상속한 뒤 모델이 명시한 scalar와 구조화 필드만
// 통째로 교체하므로, 빠진 값은 유지되고 reasoning mapping은 재귀 병합되지 않습니다.
#[test]
fn model_layer_replaces_only_explicit_fields_without_recursive_merge() {
    let base = complete_layer();
    let model = ModelProfileLayer::new(
        None,
        None,
        None,
        Some(4_096),
        Some(parameters("{effort: high}")),
        None,
        None,
        None,
    );

    let profile = EffectiveModelProfile::resolve(Some(&base), &model).unwrap();

    assert_eq!(profile.api_dialect(), ApiDialect::OpenAiResponses);
    assert_eq!(profile.context().input_token_limit(), 100_000);
    assert_eq!(profile.context().max_output_tokens(), 4_096);
    assert_eq!(profile.context().tokenizer_profile(), "utf8-bytes/v1");
    assert_eq!(
        profile.reasoning_parameters(),
        &parameters("{effort: high}")
    );
    assert_eq!(
        profile.optional_request_parameters(),
        &parameters("{parallel_tool_calls: true}")
    );
}

// base와 model을 합친 뒤에도 필수 필드가 하나라도 없거나 출력 한도가 입력 한도보다
// 작지 않으면 완전한 실행 profile을 만들지 않아 암묵적 기본값을 차단합니다.
#[test]
fn resolution_rejects_missing_fields_and_invalid_capability_limits() {
    let missing = EffectiveModelProfile::resolve(None, &ModelProfileLayer::default()).unwrap_err();
    assert_eq!(
        missing.to_string(),
        "resolved model profile is missing api_dialect"
    );

    let invalid = ModelProfileLayer::new(
        Some(ApiDialect::OpenAiResponses),
        Some(id("utf8-bytes/v1")),
        Some(100),
        Some(100),
        Some(parameters("{}")),
        Some(parameters("{}")),
        Some(id("local-tools/v1")),
        Some(id("semantic-terminal/v1")),
    );
    assert!(
        EffectiveModelProfile::resolve(None, &invalid)
            .unwrap_err()
            .to_string()
            .contains("max_output_tokens smaller")
    );
}

// profile 식별자는 닫힌 ASCII name/version 문법과 128-byte 경계를 따르므로 대문자,
// 0 version, 빈 name, 잘못된 구분자, 초과 길이를 각각 거절합니다.
#[test]
fn versioned_profile_id_enforces_the_exact_grammar_and_bound() {
    assert_eq!(id("a.b_c-d/v12").as_str(), "a.b_c-d/v12");
    for invalid in ["Upper/v1", "name/v0", "/v1", "name/1", "name/v01"] {
        assert!(VersionedProfileId::new(invalid).is_err(), "{invalid}");
    }
    let overlong = format!("{}/v1", "a".repeat(126));
    assert_eq!(overlong.len(), 129);
    assert!(VersionedProfileId::new(overlong).is_err());
}

// 구조화 값은 mapping key 순서를 무시하고 sequence 순서를 보존하며 integer와 float를
// 구분하고 -0.0을 0.0으로 정규화해 authoring과 durable round trip의 비교가 같습니다.
#[test]
fn structured_parameters_have_variant_exact_deterministic_equality() {
    assert_eq!(parameters("{b: 2, a: 1}"), parameters("{a: 1, b: 2}"));
    assert_ne!(parameters("[1, 2]"), parameters("[2, 1]"));
    assert_ne!(parameters("1"), parameters("1.0"));
    assert_eq!(parameters("-0.0"), parameters("0.0"));

    let original = parameters("{integer: 1, float: 1.0, negative: -1}");
    let encoded = serde_json::to_string(&original).unwrap();
    let decoded: ModelProfileParameters = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, original);
}

// 중복·비문자열 mapping key와 비유한 부동소수점은 parser가 값을 덮어쓰거나
// 비교 불가능한 profile을 만들기 전에 명시적으로 실패합니다.
#[test]
fn structured_parameters_reject_duplicate_keys_and_nonfinite_numbers() {
    assert!(serde_norway::from_str::<ModelProfileParameters>("{a: 1, a: 2}").is_err());
    assert!(serde_norway::from_str::<ModelProfileParameters>("{1: value}").is_err());
    assert!(serde_norway::from_str::<ModelProfileParameters>(".nan").is_err());
    assert!(serde_norway::from_str::<ModelProfileParameters>(".inf").is_err());
}
