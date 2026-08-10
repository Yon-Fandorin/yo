use super::super::{
    AccountId, ApiDialect, ConnectorId, EffectiveModelBinding, ModelId, NormalizedEndpoint,
    ProviderId,
};

// public dialect는 connector를 하나로 파생하며 durable identity가 다른 pair를 주장하면 거부한다.
#[test]
fn api_dialect_derives_exactly_one_builtin_connector_and_durable_mismatch_fails() {
    let chat = EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new("token-plan").unwrap(),
        ModelId::new("deepseek-v4-flash-0731").unwrap(),
        ApiDialect::OpenAiChatCompletions,
        NormalizedEndpoint::parse("https://dashscope-intl.aliyuncs.com/compatible-mode/v1")
            .unwrap(),
    );
    assert_eq!(
        chat.connector_id().as_str(),
        ConnectorId::OPENAI_CHAT_COMPLETIONS
    );
    assert!(
        EffectiveModelBinding::from_durable(
            chat.provider_id().clone(),
            chat.account_id().clone(),
            chat.model_id().clone(),
            ConnectorId::new(ConnectorId::OPENAI_RESPONSES).unwrap(),
            ApiDialect::OpenAiChatCompletions,
            chat.endpoint().clone(),
        )
        .is_err()
    );
}

// stable identity는 표시 이름과 분리되므로 유효한 ID를 그대로 보존하고 공백이나 control
// 문자가 섞인 설정 key는 서로 다른 routing authority가 되기 전에 거절하는지 검증합니다.
#[test]
fn validates_stable_model_service_identities() {
    assert_eq!(ProviderId::new("qwencloud").unwrap().as_str(), "qwencloud");
    assert_eq!(
        ModelId::new("qwen/qwen3.8max").unwrap().as_str(),
        "qwen/qwen3.8max"
    );
    assert!(AccountId::new(" account").is_err());
    assert!(ModelId::new("model\nname").is_err());
}

// QwenCloud Token Plan base URL은 HTTPS canonical endpoint 하나로 정규화하되 credential,
// query, fragment를 endpoint identity 안에 받아들이지 않는지 검증합니다.
#[test]
fn normalizes_the_qwencloud_endpoint_and_rejects_unsafe_components() {
    let endpoint = NormalizedEndpoint::parse(
        "https://TOKEN-PLAN.ap-southeast-1.maas.aliyuncs.com:443/compatible-mode/v1/",
    )
    .unwrap();

    assert_eq!(
        endpoint.as_str(),
        "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1"
    );
    assert!(NormalizedEndpoint::parse("http://example.com/v1").is_err());
    assert!(NormalizedEndpoint::parse("https://key@example.com/v1").is_err());
    assert!(NormalizedEndpoint::parse("https://example.com/v1?mode=x").is_err());
    assert!(NormalizedEndpoint::parse("https://example.com/v1#fragment").is_err());
}

// parse조차 할 수 없는 endpoint 입력에 secret-looking text가 포함되어도 URL parser
// 진단에는 원문이 남지 않아 잘못 배치한 credential을 로그로 유출하지 않는지 검증합니다.
#[test]
fn redacts_unparseable_endpoint_input_from_diagnostics() {
    let error = NormalizedEndpoint::parse("sk-sensitive-value://[").unwrap_err();
    let diagnostic = format!("{error:?} {error}");

    assert!(!diagnostic.contains("sk-sensitive-value"));
}
