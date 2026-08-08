use super::{AccountId, ModelCatalog, ModelId, ModelServiceError, ProviderId};

/// One exact Provider/Account/Model coordinate selected for a Yo-managed binding.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModelSelection {
    provider: ProviderId,
    account: AccountId,
    model: ModelId,
}

impl ModelSelection {
    #[must_use]
    pub const fn new(provider: ProviderId, account: AccountId, model: ModelId) -> Self {
        Self {
            provider,
            account,
            model,
        }
    }

    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    /// Stable row identity containing all three coordinates, independent of display labels.
    #[must_use]
    pub fn row_identity(&self) -> String {
        format!(
            "{}:{}|{}:{}|{}:{}",
            self.provider.as_str().len(),
            self.provider,
            self.account.as_str().len(),
            self.account,
            self.model.as_str().len(),
            self.model
        )
    }
}

/// A presentation-neutral catalog projection in Provider -> Account -> Model order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSelectionChoice {
    selection: ModelSelection,
    provider_label: String,
    account_label: String,
    model_label: String,
}

impl ModelSelectionChoice {
    #[must_use]
    pub const fn selection(&self) -> &ModelSelection {
        &self.selection
    }

    #[must_use]
    pub fn provider_label(&self) -> &str {
        &self.provider_label
    }

    #[must_use]
    pub fn account_label(&self) -> &str {
        &self.account_label
    }

    #[must_use]
    pub fn model_label(&self) -> &str {
        &self.model_label
    }
}

/// Owns direct-command resolution and exact picker acceptance outside any frontend.
#[derive(Clone, Debug)]
pub struct ModelSelectionController {
    catalog: ModelCatalog,
    current: Option<ModelSelection>,
    choices: Vec<ModelSelectionChoice>,
}

impl ModelSelectionController {
    #[must_use]
    pub fn new(catalog: ModelCatalog, current: Option<ModelSelection>) -> Self {
        let mut choices = catalog
            .entries()
            .iter()
            .map(|entry| {
                let binding = entry.binding();
                ModelSelectionChoice {
                    selection: ModelSelection::new(
                        binding.provider_id().clone(),
                        binding.account_id().clone(),
                        binding.model_id().clone(),
                    ),
                    provider_label: entry
                        .provider_display_name()
                        .unwrap_or(binding.provider_id().as_str())
                        .to_owned(),
                    account_label: entry
                        .account_display_name()
                        .unwrap_or(binding.account_id().as_str())
                        .to_owned(),
                    model_label: entry
                        .model_display_name()
                        .unwrap_or(binding.model_id().as_str())
                        .to_owned(),
                }
            })
            .collect::<Vec<_>>();
        choices.sort_by(|left, right| left.selection.cmp(&right.selection));
        Self {
            catalog,
            current,
            choices,
        }
    }

    #[must_use]
    pub fn choices(&self) -> &[ModelSelectionChoice] {
        &self.choices
    }

    #[must_use]
    pub fn current(&self) -> Option<&ModelSelection> {
        self.current.as_ref()
    }

    pub fn resolve_direct(&self, model: &ModelId) -> Result<ModelSelection, ModelServiceError> {
        let current = self.current.as_ref().ok_or_else(|| {
            ModelServiceError::new(
                "/model MODEL_ID requires a current Provider and Account; use /model to choose a complete binding",
            )
        })?;
        let selected = ModelSelection::new(
            current.provider.clone(),
            current.account.clone(),
            model.clone(),
        );
        self.accept_exact(&selected)
    }

    pub fn accept_row_identity(&self, identity: &str) -> Result<ModelSelection, ModelServiceError> {
        let mut matches = self
            .choices
            .iter()
            .filter(|choice| choice.selection.row_identity() == identity);
        let Some(choice) = matches.next() else {
            return Err(ModelServiceError::new(
                "the selected model binding is stale or no longer configured",
            ));
        };
        if matches.next().is_some() {
            return Err(ModelServiceError::new(
                "the selected model binding identity is ambiguous",
            ));
        }
        self.accept_exact(&choice.selection)
    }

    pub fn accept_exact(
        &self,
        selection: &ModelSelection,
    ) -> Result<ModelSelection, ModelServiceError> {
        self.catalog
            .resolve_model(selection.provider(), selection.account(), selection.model())?;
        Ok(selection.clone())
    }
}
