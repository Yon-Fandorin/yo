use std::collections::HashSet;

use super::{AccountId, EffectiveModelBinding, ModelId, ModelServiceError, ProviderId};

const MAX_DISPLAY_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelCatalogEntry {
    binding: EffectiveModelBinding,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
    model_display_name: Option<String>,
}

impl ModelCatalogEntry {
    pub fn new(
        binding: EffectiveModelBinding,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
        model_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        validate_display_name("Provider", provider_display_name.as_deref())?;
        validate_display_name("Account", account_display_name.as_deref())?;
        validate_display_name("Model", model_display_name.as_deref())?;
        Ok(Self {
            binding,
            provider_display_name,
            account_display_name,
            model_display_name,
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
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelCatalog {
    entries: Vec<ModelCatalogEntry>,
}

impl ModelCatalog {
    pub fn new(entries: Vec<ModelCatalogEntry>) -> Result<Self, ModelServiceError> {
        let mut bindings = HashSet::new();
        for entry in &entries {
            if !bindings.insert(entry.binding().clone()) {
                return Err(ModelServiceError::new(format!(
                    "duplicate configured model binding for Provider {}, Account {}, Model {}",
                    entry.binding().provider_id(),
                    entry.binding().account_id(),
                    entry.binding().model_id()
                )));
            }
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
