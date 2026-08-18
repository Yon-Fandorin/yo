use super::super::{
    AccountId, EffectiveModelBinding, ModelCatalog, ModelCatalogEntry, ModelContextProfile,
    ModelId, NormalizedEndpoint, ProviderId,
};

fn qwen_binding(account: &str, model: &str) -> EffectiveModelBinding {
    EffectiveModelBinding::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new(account).unwrap(),
        ModelId::new(model).unwrap(),
        "openai-responses".parse().unwrap(),
        NormalizedEndpoint::parse(
            "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/",
        )
        .unwrap(),
    )
}

// direct model 선택은 현재 Provider와 Account namespace 안에서만 exact ModelId를 찾고,
// 다른 Account에 같은 ModelId가 있어도 fallback하거나 임의 선택하지 않는지 검증합니다.
#[test]
fn resolves_models_only_inside_the_selected_provider_and_account() {
    let context = || ModelContextProfile::new(262_144, 8_192, "utf8-bytes/v1").unwrap();
    let first = ModelCatalogEntry::new(
        qwen_binding("token-a", "qwen3.8max"),
        None,
        None,
        None,
        context(),
    )
    .unwrap();
    let second = ModelCatalogEntry::new(
        qwen_binding("token-b", "qwen3.8max"),
        None,
        None,
        None,
        context(),
    )
    .unwrap();
    let catalog = ModelCatalog::new(vec![first, second]).unwrap();

    let selected = catalog
        .resolve_model(
            &ProviderId::new("qwencloud").unwrap(),
            &AccountId::new("token-b").unwrap(),
            &ModelId::new("qwen3.8max").unwrap(),
        )
        .unwrap();
    assert_eq!(selected.binding().account_id().as_str(), "token-b");
    assert_eq!(selected.context().input_token_limit(), 262_144);
    assert_eq!(selected.context().max_output_tokens(), Some(8_192));
    assert_eq!(selected.context().tokenizer_profile(), "utf8-bytes/v1");
    assert!(
        catalog
            .resolve_model(
                &ProviderId::new("qwencloud").unwrap(),
                &AccountId::new("missing").unwrap(),
                &ModelId::new("qwen3.8max").unwrap(),
            )
            .is_err()
    );
}

// flat catalog의 반복 entry가 같은 stable Provider/Account에 서로 다른 표시 이름을 붙이면
// picker마다 이름이 달라질 수 있으므로 catalog 생성 단계에서 모순을 거절합니다.
#[test]
fn rejects_inconsistent_provider_and_account_display_names() {
    let context = || ModelContextProfile::new(100, 10, "test/v1").unwrap();
    let provider_mismatch = ModelCatalog::new(vec![
        ModelCatalogEntry::new(
            qwen_binding("default", "model-a"),
            Some("QwenCloud".to_owned()),
            Some("Default".to_owned()),
            None,
            context(),
        )
        .unwrap(),
        ModelCatalogEntry::new(
            qwen_binding("default", "model-b"),
            Some("Qwen Cloud".to_owned()),
            Some("Default".to_owned()),
            None,
            context(),
        )
        .unwrap(),
    ])
    .unwrap_err();
    assert!(provider_mismatch.to_string().contains("Provider qwencloud"));

    let account_mismatch = ModelCatalog::new(vec![
        ModelCatalogEntry::new(
            qwen_binding("default", "model-a"),
            Some("QwenCloud".to_owned()),
            Some("Primary".to_owned()),
            None,
            context(),
        )
        .unwrap(),
        ModelCatalogEntry::new(
            qwen_binding("default", "model-b"),
            Some("QwenCloud".to_owned()),
            Some("Default".to_owned()),
            None,
            context(),
        )
        .unwrap(),
    ])
    .unwrap_err();
    assert!(account_mismatch.to_string().contains("Account default"));
}

// context profile은 양수 token limit, 그보다 작은 양수 reserve, 식별 가능한 tokenizer profile을
// 모두 요구함을 검증합니다.
#[test]
fn rejects_incomplete_model_context_profiles() {
    assert!(ModelContextProfile::new(0, 1, "qwen/v1").is_err());
    assert!(ModelContextProfile::new(100, 0, "qwen/v1").is_err());
    assert!(ModelContextProfile::new(100, 100, "qwen/v1").is_err());
    assert!(ModelContextProfile::new(100, 10, "").is_err());
}
