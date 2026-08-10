use std::collections::BTreeSet;

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

    pub fn resolve_reference(&self, reference: &str) -> Result<ModelSelection, ModelServiceError> {
        let mut matches = BTreeSet::new();
        for choice in &self.choices {
            let selection = choice.selection();
            let bare_is_applicable = self.current.as_ref().is_none_or(|current| {
                current.provider() == selection.provider()
                    && current.account() == selection.account()
            });
            let provider_model_matches = reference
                .strip_prefix(selection.provider().as_str())
                .and_then(|suffix| suffix.strip_prefix("::"))
                .is_some_and(|suffix| suffix == selection.model().as_str());
            let complete_coordinate_matches = reference
                .strip_prefix(selection.provider().as_str())
                .and_then(|suffix| suffix.strip_prefix(':'))
                .and_then(|suffix| suffix.strip_prefix(selection.account().as_str()))
                .and_then(|suffix| suffix.strip_prefix(':'))
                .is_some_and(|suffix| suffix == selection.model().as_str());
            if (bare_is_applicable && reference == selection.model().as_str())
                || provider_model_matches
                || complete_coordinate_matches
            {
                matches.insert(selection.clone());
            }
        }

        match matches.len() {
            1 => self.accept_exact(matches.first().expect("one reference match exists")),
            0 => Err(reference_error(
                reference,
                "is not configured",
                self.choices
                    .iter()
                    .map(|choice| choice.selection().clone())
                    .collect(),
            )),
            _ => Err(reference_error(reference, "is ambiguous", matches)),
        }
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

fn reference_error(
    reference: &str,
    outcome: &str,
    coordinates: BTreeSet<ModelSelection>,
) -> ModelServiceError {
    const MAX_DIAGNOSTIC_REFERENCE_CHARS: usize = 256;

    let mut chars = reference.chars();
    let displayed = chars
        .by_ref()
        .take(MAX_DIAGNOSTIC_REFERENCE_CHARS)
        .collect::<String>();
    let truncation = if chars.next().is_some() {
        " (truncated)"
    } else {
        ""
    };
    let mut message =
        format!("model reference {displayed:?}{truncation} {outcome}; complete coordinates:");
    if coordinates.is_empty() {
        message.push_str("\n- none configured");
    } else {
        for coordinate in coordinates {
            message.push_str(&format!(
                "\n- Provider {:?}, Account {:?}, Model {:?}",
                coordinate.provider().as_str(),
                coordinate.account().as_str(),
                coordinate.model().as_str()
            ));
        }
    }
    ModelServiceError::new(message)
}
