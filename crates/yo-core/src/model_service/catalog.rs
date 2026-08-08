use std::collections::{HashMap, HashSet, hash_map::Entry};

use serde_json::Value;

use super::{AccountId, EffectiveModelBinding, ModelId, ModelServiceError, ProviderId};

const MAX_DISPLAY_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelCatalogEntry {
    binding: EffectiveModelBinding,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
    model_display_name: Option<String>,
    context: ModelContextProfile,
}

impl ModelCatalogEntry {
    pub fn new(
        binding: EffectiveModelBinding,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
        model_display_name: Option<String>,
        context: ModelContextProfile,
    ) -> Result<Self, ModelServiceError> {
        validate_display_name("Provider", provider_display_name.as_deref())?;
        validate_display_name("Account", account_display_name.as_deref())?;
        validate_display_name("Model", model_display_name.as_deref())?;
        Ok(Self {
            binding,
            provider_display_name,
            account_display_name,
            model_display_name,
            context,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &EffectiveModelBinding {
        &self.binding
    }

    #[must_use]
    pub fn provider_display_name(&self) -> Option<&str> {
        self.provider_display_name.as_deref()
    }

    #[must_use]
    pub fn account_display_name(&self) -> Option<&str> {
        self.account_display_name.as_deref()
    }

    #[must_use]
    pub fn model_display_name(&self) -> Option<&str> {
        self.model_display_name.as_deref()
    }

    pub const fn context(&self) -> &ModelContextProfile {
        &self.context
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelContextProfile {
    input_token_limit: u64,
    max_output_tokens: u64,
    tokenizer_profile: String,
}

impl ModelContextProfile {
    pub fn new(
        input_token_limit: u64,
        max_output_tokens: u64,
        tokenizer_profile: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let tokenizer_profile = tokenizer_profile.into();
        if input_token_limit == 0
            || max_output_tokens == 0
            || max_output_tokens >= input_token_limit
            || tokenizer_profile.is_empty()
            || tokenizer_profile.len() > 128
            || !tokenizer_profile.is_ascii()
        {
            return Err(ModelServiceError::new(
                "model context profile requires a positive limit, a smaller positive max_output_tokens value, and a bounded ASCII tokenizer profile",
            ));
        }
        Ok(Self {
            input_token_limit,
            max_output_tokens,
            tokenizer_profile,
        })
    }

    pub const fn input_token_limit(&self) -> u64 {
        self.input_token_limit
    }

    pub const fn max_output_tokens(&self) -> u64 {
        self.max_output_tokens
    }

    pub fn tokenizer_profile(&self) -> &str {
        &self.tokenizer_profile
    }
}

pub trait ModelTokenCounter: Send {
    fn count_input_tokens(
        &self,
        tokenizer_profile: &str,
        request: &Value,
    ) -> Result<u64, ModelTokenCounterError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelTokenCounterError {
    message: String,
}

impl ModelTokenCounterError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ModelTokenCounterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ModelTokenCounterError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelCatalog {
    entries: Vec<ModelCatalogEntry>,
}

impl ModelCatalog {
    pub fn new(entries: Vec<ModelCatalogEntry>) -> Result<Self, ModelServiceError> {
        let mut bindings = HashSet::new();
        let mut provider_display_names = HashMap::new();
        let mut account_display_names = HashMap::new();
        for entry in &entries {
            let binding = entry.binding();
            let provider_id = binding.provider_id().clone();
            let account_id = binding.account_id().clone();
            let model_id = binding.model_id().clone();
            if !bindings.insert((provider_id.clone(), account_id.clone(), model_id)) {
                return Err(ModelServiceError::new(format!(
                    "duplicate configured model binding for Provider {}, Account {}, Model {}",
                    binding.provider_id(),
                    binding.account_id(),
                    binding.model_id()
                )));
            }
            require_consistent_display_name(
                &mut provider_display_names,
                provider_id.clone(),
                entry.provider_display_name(),
                format!("Provider {provider_id}"),
            )?;
            require_consistent_display_name(
                &mut account_display_names,
                (provider_id.clone(), account_id.clone()),
                entry.account_display_name(),
                format!("Provider {provider_id} Account {account_id}"),
            )?;
        }
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[ModelCatalogEntry] {
        &self.entries
    }

    pub fn resolve_model(
        &self,
        provider_id: &ProviderId,
        account_id: &AccountId,
        model_id: &ModelId,
    ) -> Result<&ModelCatalogEntry, ModelServiceError> {
        let mut matches = self.entries.iter().filter(|entry| {
            let binding = entry.binding();
            binding.provider_id() == provider_id
                && binding.account_id() == account_id
                && binding.model_id() == model_id
        });
        let Some(selected) = matches.next() else {
            return Err(ModelServiceError::new(format!(
                "Model {model_id} is not configured for Provider {provider_id} and Account {account_id}"
            )));
        };
        if matches.next().is_some() {
            return Err(ModelServiceError::new(format!(
                "Model {model_id} is ambiguous for Provider {provider_id} and Account {account_id}"
            )));
        }
        Ok(selected)
    }
}

fn require_consistent_display_name<K>(
    names: &mut HashMap<K, Option<String>>,
    key: K,
    value: Option<&str>,
    identity: String,
) -> Result<(), ModelServiceError>
where
    K: Eq + std::hash::Hash,
{
    let value = value.map(str::to_owned);
    match names.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        },
        Entry::Occupied(entry) if entry.get() == &value => Ok(()),
        Entry::Occupied(_) => Err(ModelServiceError::new(format!(
            "inconsistent display name for {identity}"
        ))),
    }
}

fn validate_display_name(
    label: &'static str,
    value: Option<&str>,
) -> Result<(), ModelServiceError> {
    if let Some(value) = value
        && (value.is_empty()
            || value.len() > MAX_DISPLAY_NAME_BYTES
            || value.chars().any(char::is_control))
    {
        return Err(ModelServiceError::new(format!(
            "{label} display name must contain 1 to {MAX_DISPLAY_NAME_BYTES} bytes without control characters"
        )));
    }
    Ok(())
}
