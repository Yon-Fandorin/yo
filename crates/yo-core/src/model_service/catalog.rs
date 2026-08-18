use std::collections::{HashMap, HashSet, hash_map::Entry};

use serde_json::Value;

use super::{
    AccountId, CompleteModelBinding, EffectiveModelBinding, EffectiveModelProfile, ModelId,
    ModelLastFailure, ModelServiceError, ProviderId, VersionedProfileId,
};

const MAX_DISPLAY_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelCatalogEntry {
    binding: CatalogBinding,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
    model_display_name: Option<String>,
    last_failure: Option<ModelLastFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CatalogBinding {
    Legacy {
        binding: EffectiveModelBinding,
        context: ModelContextProfile,
    },
    Complete(CompleteModelBinding),
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
            binding: CatalogBinding::Legacy { binding, context },
            provider_display_name,
            account_display_name,
            model_display_name,
            last_failure: None,
        })
    }

    pub fn with_explicit_profile(
        binding: EffectiveModelBinding,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
        model_display_name: Option<String>,
        profile: EffectiveModelProfile,
    ) -> Result<Self, ModelServiceError> {
        let binding = CompleteModelBinding::new(binding, profile)?;
        validate_display_name("Provider", provider_display_name.as_deref())?;
        validate_display_name("Account", account_display_name.as_deref())?;
        validate_display_name("Model", model_display_name.as_deref())?;
        Ok(Self {
            provider_display_name,
            account_display_name,
            model_display_name,
            last_failure: None,
            binding: CatalogBinding::Complete(binding),
        })
    }

    pub(crate) fn from_stored(
        binding: CompleteModelBinding,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
        model_display_name: Option<String>,
        last_failure: Option<ModelLastFailure>,
    ) -> Result<Self, ModelServiceError> {
        validate_display_name("Provider", provider_display_name.as_deref())?;
        validate_display_name("Account", account_display_name.as_deref())?;
        validate_display_name("Model", model_display_name.as_deref())?;
        Ok(Self {
            binding: CatalogBinding::Complete(binding),
            provider_display_name,
            account_display_name,
            model_display_name,
            last_failure,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &EffectiveModelBinding {
        match &self.binding {
            CatalogBinding::Legacy { binding, .. } => binding,
            CatalogBinding::Complete(binding) => binding.binding(),
        }
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

    #[must_use]
    pub const fn last_failure(&self) -> Option<&ModelLastFailure> {
        self.last_failure.as_ref()
    }

    pub const fn context(&self) -> &ModelContextProfile {
        match &self.binding {
            CatalogBinding::Legacy { context, .. } => context,
            CatalogBinding::Complete(binding) => binding.profile().context(),
        }
    }

    #[must_use]
    pub const fn explicit_profile(&self) -> Option<&EffectiveModelProfile> {
        match &self.binding {
            CatalogBinding::Legacy { .. } => None,
            CatalogBinding::Complete(binding) => Some(binding.profile()),
        }
    }

    #[must_use]
    pub const fn complete_binding(&self) -> Option<&CompleteModelBinding> {
        match &self.binding {
            CatalogBinding::Legacy { .. } => None,
            CatalogBinding::Complete(binding) => Some(binding),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelContextProfile {
    input_token_limit: u64,
    max_output_tokens: Option<u64>,
    tokenizer_profile: VersionedProfileId,
}

impl ModelContextProfile {
    pub fn new(
        input_token_limit: u64,
        max_output_tokens: u64,
        tokenizer_profile: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let tokenizer_profile = VersionedProfileId::new(tokenizer_profile)?;
        Self::from_versioned(
            input_token_limit,
            Some(max_output_tokens),
            tokenizer_profile,
        )
    }

    pub fn with_optional_output_limit(
        input_token_limit: u64,
        max_output_tokens: Option<u64>,
        tokenizer_profile: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let tokenizer_profile = VersionedProfileId::new(tokenizer_profile)?;
        Self::from_versioned(input_token_limit, max_output_tokens, tokenizer_profile)
    }

    pub(super) fn from_versioned(
        input_token_limit: u64,
        max_output_tokens: Option<u64>,
        tokenizer_profile: VersionedProfileId,
    ) -> Result<Self, ModelServiceError> {
        if input_token_limit == 0
            || max_output_tokens.is_some_and(|value| value == 0 || value >= input_token_limit)
        {
            return Err(ModelServiceError::new(
                "model context profile requires a positive input limit and any known max_output_tokens to be positive and smaller than input_token_limit",
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

    pub const fn max_output_tokens(&self) -> Option<u64> {
        self.max_output_tokens
    }

    pub fn tokenizer_profile(&self) -> &str {
        self.tokenizer_profile.as_str()
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

pub(super) fn validate_display_name(
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
