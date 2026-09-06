use serde_json::{Value, json};
use yo_core::{AdmittedReplayProfile, AdmittedToolPolicy, ReasoningEffort};

use super::*;

fn kimi_definition() -> Value {
    json!({
        "provider": "kimi", "account": "default", "model": "kimi-k3",
        "connector": "kimi-chat-completions", "api_dialect": "kimi-chat-completions",
        "base_url": "https://api.moonshot.ai/v1", "tokenizer_profile": "utf8-bytes/v1",
        "input_token_limit": 1048576, "max_output_tokens": 131072,
        "reasoning_parameters": {"effort": "max"}, "optional_request_parameters": {},
        "tool_capability_policy": "local-tools/v1", "replay_profile": "kimi-private-local-plaintext/v1"
    })
}

fn complete(value: &Value) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&value.to_string()).unwrap()
}

// 세 exact native route가 각 owner의 검증 결과를 반환하며 Kimi를 generic admission으로
// 우회하지 않고 connector의 reasoning/tool/replay 결과를 그대로 전달하는지 검증합니다.
#[test]
fn admits_every_exact_native_route_without_generic_kimi_fallback() {
    for dialect in ["openai-responses", "openai-chat-completions"] {
        let mut value = kimi_definition();
        value["provider"] = json!("vendor");
        value["model"] = json!("model");
        value["connector"] = json!(dialect);
        value["api_dialect"] = json!(dialect);
        value["base_url"] = json!("https://example.test/v1");
        value["reasoning_parameters"] = json!({"effort": "medium"});
        value["tool_capability_policy"] = json!("no-tools/v1");
        value["replay_profile"] = json!("semantic-only/v1");
        let admitted = NativeBindingAdmission.admit(&complete(&value)).unwrap();
        assert_eq!(
            admitted.profile().reasoning_effort(),
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(
            admitted.profile().tool_policy(),
            AdmittedToolPolicy::NoTools
        );
        assert_eq!(
            admitted.replay_profile(),
            AdmittedReplayProfile::SemanticOnly
        );
    }
    let binding = complete(&kimi_definition());
    assert!(yo_core::admit_standard_complete_binding(&binding).is_err());
    let admitted = NativeBindingAdmission.admit(&binding).unwrap();
    assert_eq!(
        admitted.profile().reasoning_effort(),
        Some(ReasoningEffort::Max)
    );
    assert_eq!(
        admitted.profile().tool_policy(),
        AdmittedToolPolicy::LocalTools
    );
    assert_eq!(
        admitted.replay_profile(),
        AdmittedReplayProfile::ProviderPrivateLocalPlaintext
    );
}

// Provider, endpoint/model tuple, tokenizer와 replay가 Kimi route의 닫힌 조건을 벗어나면
// 다른 connector나 generic profile로 fallback하지 않고 실패하는지 검증합니다.
#[test]
fn rejects_cross_provider_and_cross_product_kimi_profiles_without_fallback() {
    for (field, value) in [
        ("provider", json!("vendor")),
        ("base_url", json!("https://api.kimi.com/coding/v1")),
        ("model", json!("k3")),
        ("tokenizer_profile", json!("other/v1")),
        ("replay_profile", json!("semantic-only/v1")),
        ("max_output_tokens", json!(131071)),
        ("reasoning_parameters", json!({"effort": "medium"})),
    ] {
        let mut definition = kimi_definition();
        definition[field] = value;
        assert!(
            NativeBindingAdmission
                .admit(&complete(&definition))
                .is_err(),
            "{field}"
        );
    }
    let mut generic_kimi = kimi_definition();
    generic_kimi["connector"] = json!("openai-responses");
    generic_kimi["api_dialect"] = json!("openai-responses");
    generic_kimi["reasoning_parameters"] = json!({});
    generic_kimi["replay_profile"] = json!("semantic-only/v1");
    assert!(
        NativeBindingAdmission
            .admit(&complete(&generic_kimi))
            .is_err()
    );
}

// durable identity의 unknown route와 Connector/dialect 불일치는 dispatcher에 도착하기
// 전에 core의 단일 identity 규칙으로 닫혀 대체 route로 해석되지 않는지 검증합니다.
#[test]
fn rejects_missing_and_mismatched_durable_routes_before_dispatch() {
    for connector in ["missing-connector", "openai-responses"] {
        let mut value = kimi_definition();
        value["connector"] = json!(connector);
        assert!(CompleteModelBinding::from_durable_json(&value.to_string()).is_err());
    }
}
