use std::collections::BTreeMap;

use super::{
    super::{
        AccountId, HostId, ModelCatalog, ModelId, ModelLastFailure, ModelServiceError, ProviderId,
    },
    HostModelCatalog, HostModelSelection, ModelPickerTarget, ModelSelection,
};

/// Presentation-neutral account section in the unified model picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelPickerSection {
    identity: String,
    label: String,
    status: Option<String>,
    choices: Vec<ModelPickerChoice>,
}

/// Presentation-neutral selectable row in one account section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelPickerChoice {
    target: ModelPickerTarget,
    label: String,
    detail: String,
    current: bool,
    enabled: bool,
    disabled_reason: Option<String>,
}

/// A presentation-neutral catalog projection in Provider -> Account -> Model order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelSelectionChoice {
    selection: ModelSelection,
    provider_label: String,
    account_label: String,
    model_label: String,
    last_failure: Option<ModelLastFailure>,
    enabled: bool,
}

/// Owns direct-command resolution and exact picker acceptance outside any frontend.
#[derive(Clone, Debug)]
pub struct ModelSelectionController {
    pub(super) catalog: ModelCatalog,
    pub(super) current: Option<ModelSelection>,
    pub(super) choices: Vec<ModelSelectionChoice>,
    pub(super) sections: Vec<ModelPickerSection>,
}

impl ModelPickerSection {
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    #[must_use]
    pub fn choices(&self) -> &[ModelPickerChoice] {
        &self.choices
    }

    fn is_current(&self) -> bool {
        self.choices.iter().any(ModelPickerChoice::is_current)
    }
}

impl ModelPickerChoice {
    #[must_use]
    pub const fn target(&self) -> &ModelPickerTarget {
        &self.target
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    #[must_use]
    pub const fn is_current(&self) -> bool {
        self.current
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn disabled_reason(&self) -> Option<&str> {
        self.disabled_reason.as_deref()
    }
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

    #[must_use]
    pub const fn last_failure(&self) -> Option<&ModelLastFailure> {
        self.last_failure.as_ref()
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn disabled_reason(&self) -> Option<&'static str> {
        if self.enabled {
            None
        } else {
            Some("disabled by operator")
        }
    }
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
                    last_failure: entry.last_failure().cloned(),
                    enabled: entry.is_enabled(),
                }
            })
            .collect::<Vec<_>>();
        choices.sort_by(|left, right| left.selection.cmp(&right.selection));
        let mut controller = Self {
            catalog,
            current,
            choices,
            sections: Vec::new(),
        };
        controller.rebuild_managed_sections();
        controller
    }

    #[must_use]
    pub fn choices(&self) -> &[ModelSelectionChoice] {
        &self.choices
    }

    #[must_use]
    pub fn current(&self) -> Option<&ModelSelection> {
        self.current.as_ref()
    }

    /// Adds one fresh host inventory. `active` marks only the live host account's exact current
    /// model; inventories from other available hosts remain ordinary sections.
    pub fn with_host_catalog(self, catalog: HostModelCatalog, active: bool) -> Self {
        let current = active.then(|| catalog.current_model.clone()).flatten();
        self.with_host_catalog_state(catalog, active, current.as_ref(), None)
    }

    /// Adds one host inventory with runtime availability and an exact live-model override.
    pub fn with_host_catalog_state(
        mut self,
        catalog: HostModelCatalog,
        active: bool,
        active_model: Option<&ModelId>,
        unavailable_reason: Option<&str>,
    ) -> Self {
        if active {
            self.current = None;
            for section in &mut self.sections {
                for choice in &mut section.choices {
                    if choice.current {
                        choice.current = false;
                        if let Some(label) = choice.label.strip_suffix(" (current)") {
                            choice.label = label.to_owned();
                        }
                    }
                    choice.enabled = false;
                    choice.disabled_reason = Some("semantic handoff is not implemented".to_owned());
                }
            }
        }
        let section_identity = host_section_identity(&catalog.host, &catalog.account);
        self.sections
            .retain(|section| section.identity != section_identity);
        let choices = catalog
            .models
            .into_iter()
            .map(|model| {
                let current = active && active_model == Some(&model.id);
                let disabled_reason = model
                    .unavailable_reason
                    .or_else(|| unavailable_reason.map(str::to_owned));
                let target = ModelPickerTarget::Host(HostModelSelection::new(
                    catalog.host.clone(),
                    catalog.account.clone(),
                    model.id.clone(),
                    catalog.revision.clone(),
                ));
                ModelPickerChoice {
                    target,
                    label: if current {
                        format!("{} (current)", model.label)
                    } else {
                        model.label
                    },
                    detail: model.id.to_string(),
                    current,
                    enabled: model.selectable && disabled_reason.is_none(),
                    disabled_reason,
                }
            })
            .collect();
        self.sections.push(ModelPickerSection {
            identity: section_identity,
            label: format!("{} · {}", catalog.host_label, catalog.account_label),
            status: None,
            choices,
        });
        self.sort_sections();
        self
    }

    /// Adds a non-selectable account-local status without suppressing sibling accounts.
    pub fn with_host_status(
        mut self,
        host: &HostId,
        host_label: impl Into<String>,
        account: &AccountId,
        account_label: impl Into<String>,
        status: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let host_label = host_label.into();
        let account_label = account_label.into();
        let status = status.into();
        super::validate_picker_text("host label", &host_label)?;
        super::validate_picker_text("host account label", &account_label)?;
        super::validate_picker_text("host account status", &status)?;
        let identity = host_section_identity(host, account);
        self.sections.retain(|section| section.identity != identity);
        self.sections.push(ModelPickerSection {
            identity,
            label: format!("{host_label} · {account_label}"),
            status: Some(status),
            choices: Vec::new(),
        });
        self.sort_sections();
        Ok(self)
    }

    #[must_use]
    pub fn sections(&self) -> &[ModelPickerSection] {
        &self.sections
    }

    pub fn accept_picker_identity(
        &self,
        identity: &str,
    ) -> Result<ModelPickerTarget, ModelServiceError> {
        let matches = self
            .sections
            .iter()
            .flat_map(ModelPickerSection::choices)
            .filter(|choice| choice.target.row_identity() == identity)
            .collect::<Vec<_>>();
        let [choice] = matches.as_slice() else {
            return Err(ModelServiceError::new(
                "the selected model row is stale or ambiguous",
            ));
        };
        if !choice.enabled {
            return Err(ModelServiceError::new(
                choice
                    .disabled_reason
                    .as_deref()
                    .unwrap_or("the selected model is unavailable"),
            ));
        }
        match &choice.target {
            ModelPickerTarget::Managed(selection) => {
                self.accept_exact(selection).map(ModelPickerTarget::Managed)
            },
            ModelPickerTarget::Host(selection) => Ok(ModelPickerTarget::Host(selection.clone())),
        }
    }

    fn rebuild_managed_sections(&mut self) {
        let mut grouped = BTreeMap::<(String, String, ProviderId, AccountId), Vec<_>>::new();
        for choice in &self.choices {
            grouped
                .entry((
                    choice.provider_label.clone(),
                    choice.account_label.clone(),
                    choice.selection.provider().clone(),
                    choice.selection.account().clone(),
                ))
                .or_default()
                .push(choice);
        }
        self.sections = grouped
            .into_iter()
            .map(
                |((provider_label, account_label, provider, account), choices)| {
                    let choices = choices
                        .into_iter()
                        .map(|choice| {
                            let current = self.current.as_ref() == Some(&choice.selection);
                            ModelPickerChoice {
                                target: ModelPickerTarget::Managed(choice.selection.clone()),
                                label: if current {
                                    format!("{} (current)", choice.model_label)
                                } else {
                                    choice.model_label.clone()
                                },
                                detail: choice.last_failure.as_ref().map_or_else(
                                    || choice.selection.model().to_string(),
                                    |failure| {
                                        format!(
                                            "warning: {} at {}",
                                            failure.kind(),
                                            failure.observed_at()
                                        )
                                    },
                                ),
                                current,
                                enabled: choice.enabled,
                                disabled_reason: choice.disabled_reason().map(str::to_owned),
                            }
                        })
                        .collect();
                    ModelPickerSection {
                        identity: managed_section_identity(&provider, &account),
                        label: format!("{provider_label} · {account_label}"),
                        status: None,
                        choices,
                    }
                },
            )
            .collect();
        self.sort_sections();
    }

    fn sort_sections(&mut self) {
        self.sections.sort_by(|left, right| {
            right
                .is_current()
                .cmp(&left.is_current())
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.identity.cmp(&right.identity))
        });
    }
}

fn managed_section_identity(provider: &ProviderId, account: &AccountId) -> String {
    format!(
        "section|managed:{}:{}|account:{}:{}",
        provider.as_str().len(),
        provider,
        account.as_str().len(),
        account,
    )
}

fn host_section_identity(host: &HostId, account: &AccountId) -> String {
    format!(
        "section|host:{}:{}|account:{}:{}",
        host.as_str().len(),
        host.as_str(),
        account.as_str().len(),
        account,
    )
}
