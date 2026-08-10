use super::super::{
    AccountId, ApiDialect, EffectiveModelBinding, ModelCatalogEntry, ModelContextProfile, ModelId,
    NormalizedEndpoint, ProviderId,
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
