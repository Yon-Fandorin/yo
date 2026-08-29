use super::super::{
    AccountId, ApiDialect, CompleteModelBinding, EffectiveModelBinding, ModelCatalogEntry,
    ModelContextProfile, ModelId, NormalizedEndpoint, ProviderId,
};

pub(super) fn selection_entry(
    provider: &str,
    account: &str,
    model: &str,
    provider_label: &str,
    account_label: &str,
) -> ModelCatalogEntry {
    ModelCatalogEntry::new(
        EffectiveModelBinding::new(
            ProviderId::new(provider).unwrap(),
            AccountId::new(account).unwrap(),
            ModelId::new(model).unwrap(),
            ApiDialect::OpenAiResponses,
            NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
        ),
        Some(provider_label.to_owned()),
        Some(account_label.to_owned()),
        None,
        ModelContextProfile::new(1_000, 100, "utf8-bytes/v1").unwrap(),
    )
    .unwrap()
}

pub(super) fn disabled_selection_entry(
    provider: &str,
    account: &str,
    model: &str,
) -> ModelCatalogEntry {
    let durable = format!(
        r#"{{"provider":"{provider}","account":"{account}","model":"{model}","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{{}},"optional_request_parameters":{{}},"tool_capability_policy":"local-tools/v1"}}"#
    );
    ModelCatalogEntry::from_stored(
        CompleteModelBinding::from_durable_json(&durable).unwrap(),
        Some(provider.to_owned()),
        Some(account.to_owned()),
        Some(model.to_owned()),
        None,
        false,
    )
    .unwrap()
}
