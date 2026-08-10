use yo_core::{ModelCatalogEntry, ModelTokenCounter, ModelTokenCounterError};

use crate::AppError;

pub(super) const O200K_PROFILE: &str = "o200k_base/v1";
pub(super) const UTF8_BYTES_PROFILE: &str = "utf8-bytes/v1";

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
    pub(super) fn supports(profile: &str) -> bool {
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
