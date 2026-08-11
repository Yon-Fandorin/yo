use yo_core::{ModelCatalogEntry, ModelTokenCounter, ModelTokenCounterError};

use crate::AppError;

const O200K_PROFILE: &str = "o200k_base/v1";
const UTF8_BYTES_PROFILE: &str = "utf8-bytes/v1";

pub(super) fn require_supported_tokenizer(entry: &ModelCatalogEntry) -> Result<(), AppError> {
    if TokenizerRegistry::supports(entry.context().tokenizer_profile()) {
        Ok(())
    } else {
        Err(AppError::many([format!(
            "unsupported tokenizer profile {:?}; this build supports {O200K_PROFILE} and {UTF8_BYTES_PROFILE}",
            entry.context().tokenizer_profile()
        )]))
    }
}

pub(super) struct TokenizerRegistry;

impl TokenizerRegistry {
    fn supports(profile: &str) -> bool {
        matches!(profile, O200K_PROFILE | UTF8_BYTES_PROFILE)
    }
}

impl ModelTokenCounter for TokenizerRegistry {
    fn count_input_tokens(
        &self,
        tokenizer_profile: &str,
        request: &serde_json::Value,
    ) -> Result<u64, ModelTokenCounterError> {
        let encoded = serde_json::to_string(request)
            .map_err(|_| ModelTokenCounterError::new("request cannot be tokenized"))?;
        let count = match tokenizer_profile {
            O200K_PROFILE => tiktoken_rs::o200k_base_singleton()
                .encode_with_special_tokens(&encoded)
                .len(),
            // This profile deliberately admits one token per serialized UTF-8 byte. It is a
            // conservative, provider-neutral bound for byte-backed tokenizer families when an
            // exact built-in tokenizer is unavailable; the profile name makes that policy
            // explicit rather than claiming Qwen or another model's private tokenizer.
            UTF8_BYTES_PROFILE => encoded.len(),
            _ => return Err(ModelTokenCounterError::new("unsupported tokenizer profile")),
        };
        u64::try_from(count).map_err(|_| ModelTokenCounterError::new("token count exceeds u64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection_catalog_with_tokenizer(
        entries: &[(&str, &str, &str)],
        tokenizer_profile: &str,
    ) -> yo_core::ModelCatalog {
        yo_core::ModelCatalog::new(
            entries
                .iter()
                .map(|(provider, account, model)| {
                    yo_core::ModelCatalogEntry::new(
                        yo_core::EffectiveModelBinding::new(
                            yo_core::ProviderId::new(*provider).unwrap(),
                            yo_core::AccountId::new(*account).unwrap(),
                            yo_core::ModelId::new(*model).unwrap(),
                            yo_core::ApiDialect::OpenAiResponses,
                            yo_core::NormalizedEndpoint::parse("https://example.test/v1").unwrap(),
                        ),
                        None,
                        None,
                        None,
                        yo_core::ModelContextProfile::new(1_000, 100, tokenizer_profile).unwrap(),
                    )
                    .unwrap()
                })
                .collect(),
        )
        .unwrap()
    }

    // tokenizer profile은 versioned allowlist로 해석하며 알 수 없는 이름은 추측하지 않는다.
    #[test]
    fn tokenizer_registry_is_versioned_and_fails_closed() {
        let payload = serde_json::json!({"input": "안녕", "tools": []});
        let registry = TokenizerRegistry;

        assert!(TokenizerRegistry::supports(O200K_PROFILE));
        assert!(TokenizerRegistry::supports(UTF8_BYTES_PROFILE));
        assert!(!TokenizerRegistry::supports("qwen/latest"));
        assert_eq!(
            registry
                .count_input_tokens(UTF8_BYTES_PROFILE, &payload)
                .unwrap(),
            serde_json::to_string(&payload).unwrap().len() as u64
        );
        assert!(
            registry
                .count_input_tokens("qwen/latest", &payload)
                .is_err()
        );
    }

    // catalog entry가 지원하지 않는 profile을 선언하면 실제 profile과 현재 build의
    // 전체 allowlist를 함께 노출하는 exact diagnostic으로 fail closed 한다.
    #[test]
    fn unsupported_tokenizer_profile_reports_exact_profile_and_allowlist() {
        let catalog = selection_catalog_with_tokenizer(
            &[("qwencloud", "default", "qwen3.8-max")],
            "qwen/latest",
        );

        let error = require_supported_tokenizer(&catalog.entries()[0]).unwrap_err();

        assert_eq!(
            error.to_string(),
            "unsupported tokenizer profile \"qwen/latest\"; this build supports o200k_base/v1 and utf8-bytes/v1"
        );
    }
}
