use std::collections::BTreeSet;

use super::super::{AccountId, HostId, ModelId, ModelServiceError};

/// One runtime-advertised host model. Hidden models are omitted by the adapter before this seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCatalogModel {
    pub(super) id: ModelId,
    pub(super) label: String,
    pub(super) selectable: bool,
    pub(super) unavailable_reason: Option<String>,
}

/// Fresh authenticated model inventory for one delegated host account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostModelCatalog {
    pub(super) host: HostId,
    pub(super) host_label: String,
    pub(super) account: AccountId,
    pub(super) account_label: String,
    pub(super) revision: String,
    pub(super) current_model: Option<ModelId>,
    pub(super) models: Vec<HostCatalogModel>,
}

impl HostCatalogModel {
    pub fn selectable(id: ModelId, label: impl Into<String>) -> Result<Self, ModelServiceError> {
        let label = label.into();
        super::validate_picker_text("host model label", &label)?;
        Ok(Self {
            id,
            label,
            selectable: true,
            unavailable_reason: None,
        })
    }

    pub fn unavailable(
        id: ModelId,
        label: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let label = label.into();
        let reason = reason.into();
        super::validate_picker_text("host model label", &label)?;
        super::validate_picker_text("host model unavailable reason", &reason)?;
        Ok(Self {
            id,
            label,
            selectable: false,
            unavailable_reason: Some(reason),
        })
    }

    #[must_use]
    pub const fn id(&self) -> &ModelId {
        &self.id
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn is_selectable(&self) -> bool {
        self.selectable
    }

    #[must_use]
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }
}

impl HostModelCatalog {
    pub fn new(
        host: HostId,
        host_label: impl Into<String>,
        account: AccountId,
        account_label: impl Into<String>,
        revision: impl Into<String>,
        current_model: Option<ModelId>,
        models: Vec<HostCatalogModel>,
    ) -> Result<Self, ModelServiceError> {
        let host_label = host_label.into();
        let account_label = account_label.into();
        let revision = revision.into();
        super::validate_picker_text("host label", &host_label)?;
        super::validate_picker_text("host account label", &account_label)?;
        super::validate_picker_text("host catalog revision", &revision)?;
        if models.is_empty() {
            return Err(ModelServiceError::new(
                "a host model catalog must contain at least one visible model",
            ));
        }
        let mut ids = BTreeSet::new();
        for model in &models {
            if !ids.insert(model.id.clone()) {
                return Err(ModelServiceError::new(
                    "a host model catalog contains a duplicate model id",
                ));
            }
        }
        if current_model
            .as_ref()
            .is_some_and(|current| !ids.contains(current))
        {
            return Err(ModelServiceError::new(
                "the current host model is absent from the visible catalog",
            ));
        }
        Ok(Self {
            host,
            host_label,
            account,
            account_label,
            revision,
            current_model,
            models,
        })
    }

    #[must_use]
    pub const fn host(&self) -> &HostId {
        &self.host
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    #[must_use]
    pub fn revision(&self) -> &str {
        &self.revision
    }

    #[must_use]
    pub const fn current_model(&self) -> Option<&ModelId> {
        self.current_model.as_ref()
    }

    #[must_use]
    pub fn models(&self) -> &[HostCatalogModel] {
        &self.models
    }
}
