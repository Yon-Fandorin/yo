use super::super::{
    AccountId, CompleteModelBinding, EffectiveModelBinding, ManagedConnectionAccount,
    ManagedConnectionBinding, ModelCatalog, ModelCatalogEntry, ModelCatalogProvenance,
    ModelContextProfile, ModelId, NormalizedEndpoint, ProviderId,
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

fn complete_binding(account: &str, model: &str, effort: &str) -> CompleteModelBinding {
    CompleteModelBinding::from_durable_json(&format!(
        r#"{{"provider":"qwencloud","account":"{account}","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{"effort":"{effort}"}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    ))
    .unwrap()
}

fn explicit_entry(
    account: &str,
    model: &str,
    effort: &str,
    provider_display: Option<&str>,
    account_display: Option<&str>,
    model_display: Option<&str>,
) -> ModelCatalogEntry {
    let complete = complete_binding(account, model, effort);
    ModelCatalogEntry::with_explicit_profile(
        complete.binding().clone(),
        provider_display.map(str::to_owned),
        account_display.map(str::to_owned),
        model_display.map(str::to_owned),
        complete.profile().clone(),
    )
    .unwrap()
}

fn managed_account(account: &str, provider_display: &str) -> ManagedConnectionAccount {
    ManagedConnectionAccount::new(
        ProviderId::new("qwencloud").unwrap(),
        AccountId::new(account).unwrap(),
        Some(provider_display.to_owned()),
        Some(format!("Managed {account}")),
    )
    .unwrap()
}

fn managed_binding(account: &str, model: &str, effort: &str) -> ManagedConnectionBinding {
    ManagedConnectionBinding::new(
        complete_binding(account, model, effort),
        Some(format!("Managed {model}")),
    )
    .unwrap()
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
    assert_eq!(selected.context().max_output_tokens(), 8_192);
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

// managed-only complete binding은 account 표시 정보와 함께 catalog entry가 되고 provenance가
// Managed로 남아 이후 disconnect가 수동 설정으로 오인하지 않음을 검증합니다.
#[test]
fn composes_managed_only_binding_with_managed_provenance() {
    let catalog = ModelCatalog::default()
        .compose_managed(
            &[managed_account("default", "Managed Qwen")],
            &[managed_binding("default", "model-a", "medium")],
        )
        .unwrap();

    let entry = catalog
        .resolve_model(
            &ProviderId::new("qwencloud").unwrap(),
            &AccountId::new("default").unwrap(),
            &ModelId::new("model-a").unwrap(),
        )
        .unwrap();
    assert_eq!(entry.provenance(), ModelCatalogProvenance::Managed);
    assert_eq!(entry.provider_display_name(), Some("Managed Qwen"));
    assert_eq!(entry.account_display_name(), Some("Managed default"));
    assert_eq!(entry.model_display_name(), Some("Managed model-a"));
}

// 같은 complete binding의 manual/managed provenance는 한 entry로 합쳐지고 표시 이름은
// manual 값을 우선하되 manual에 없는 model 표시는 managed 값으로 보충합니다.
#[test]
fn equal_manual_and_managed_bindings_coalesce_with_manual_display_precedence() {
    let manual = ModelCatalog::new(vec![explicit_entry(
        "default",
        "model-a",
        "medium",
        Some("Manual Qwen"),
        Some("Manual Account"),
        None,
    )])
    .unwrap();
    let catalog = manual
        .compose_managed(
            &[managed_account("default", "Managed Qwen")],
            &[managed_binding("default", "model-a", "medium")],
        )
        .unwrap();

    assert_eq!(catalog.entries().len(), 1);
    let entry = &catalog.entries()[0];
    assert_eq!(entry.provenance(), ModelCatalogProvenance::ManualAndManaged);
    assert_eq!(entry.provider_display_name(), Some("Manual Qwen"));
    assert_eq!(entry.account_display_name(), Some("Manual Account"));
    assert_eq!(entry.model_display_name(), Some("Managed model-a"));
}

// 같은 coordinate의 resolved profile이 다르면 어느 provenance도 선택하지 않고 stable한
// non-secret field 목록을 가진 BindingConflict로 전체 합성을 실패합니다.
#[test]
fn unequal_manual_and_managed_bindings_report_exact_non_secret_fields() {
    let manual = ModelCatalog::new(vec![explicit_entry(
        "default", "model-a", "medium", None, None, None,
    )])
    .unwrap();
    let error = manual
        .compose_managed(
            &[managed_account("default", "Managed Qwen")],
            &[managed_binding("default", "model-a", "high")],
        )
        .unwrap_err();

    assert_eq!(error.provider().as_str(), "qwencloud");
    assert_eq!(error.account().as_str(), "default");
    assert_eq!(error.model().as_str(), "model-a");
    assert_eq!(error.differing_fields(), &["reasoning_parameters"]);
    assert!(!error.to_string().contains("credential"));
}

// 같은 Provider의 다른 managed account도 presentation에서는 manual Provider 표시 이름을
// 공유해 출처마다 provider label이 갈라지지 않습니다.
#[test]
fn manual_provider_display_wins_for_managed_only_sibling_accounts() {
    let manual = ModelCatalog::new(vec![explicit_entry(
        "manual",
        "manual-model",
        "medium",
        Some("Manual Qwen"),
        Some("Manual Account"),
        None,
    )])
    .unwrap();
    let catalog = manual
        .compose_managed(
            &[managed_account("managed", "Managed Qwen")],
            &[managed_binding("managed", "managed-model", "medium")],
        )
        .unwrap();
    let managed = catalog
        .resolve_model(
            &ProviderId::new("qwencloud").unwrap(),
            &AccountId::new("managed").unwrap(),
            &ModelId::new("managed-model").unwrap(),
        )
        .unwrap();

    assert_eq!(managed.provider_display_name(), Some("Manual Qwen"));
    assert_eq!(managed.account_display_name(), Some("Managed managed"));
}
