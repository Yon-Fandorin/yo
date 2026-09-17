use super::{
    super::super::catalog::validate_display_name, failure::ModelLastFailure,
    reject_new_host_provider,
};
use crate::{CompleteModelBinding, ModelSelection, ModelServiceError};

/// durable stored model binding 하나와 그 model 표시 메타데이터입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredModelBinding {
    complete: CompleteModelBinding,
    model_display_name: Option<String>,
    enabled: bool,
    last_failure: Option<ModelLastFailure>,
}

impl StoredModelBinding {
    pub fn new(
        complete: CompleteModelBinding,
        model_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        reject_new_host_provider(complete.binding().provider_id())?;
        Self::from_durable(complete, model_display_name)
    }

    pub(in super::super::super) fn from_durable(
        complete: CompleteModelBinding,
        model_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        validate_display_name("Model", model_display_name.as_deref())?;
        Ok(Self {
            complete,
            model_display_name,
            enabled: true,
            last_failure: None,
        })
    }

    pub(in super::super::super) fn from_durable_with_state(
        complete: CompleteModelBinding,
        model_display_name: Option<String>,
        enabled: bool,
        last_failure: Option<ModelLastFailure>,
    ) -> Result<Self, ModelServiceError> {
        let mut stored = Self::from_durable(complete, model_display_name)?;
        stored.enabled = enabled;
        stored.last_failure = last_failure;
        Ok(stored)
    }

    #[must_use]
    pub const fn complete(&self) -> &CompleteModelBinding {
        &self.complete
    }

    #[must_use]
    pub fn model_display_name(&self) -> Option<&str> {
        self.model_display_name.as_deref()
    }

    #[must_use]
    pub const fn last_failure(&self) -> Option<&ModelLastFailure> {
        self.last_failure.as_ref()
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(in super::super::super) fn with_last_failure(
        mut self,
        last_failure: Option<ModelLastFailure>,
    ) -> Self {
        self.last_failure = last_failure;
        self
    }

    pub(in super::super::super) fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub fn selection(&self) -> ModelSelection {
        let binding = self.complete.binding();
        ModelSelection::new(
            binding.provider_id().clone(),
            binding.account_id().clone(),
            binding.model_id().clone(),
        )
    }
}
