use std::collections::{HashMap, HashSet, hash_map::Entry};

use super::super::{
    AccountId, CompleteModelBinding, ModelSelection, ModelServiceError, ProviderId,
    catalog::validate_display_name,
};

/// One durable stored account and its optional presentation metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionAccount {
    provider_id: ProviderId,
    account_id: AccountId,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
}

impl ConnectionAccount {
    pub fn new(
        provider_id: ProviderId,
        account_id: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        reject_new_host_provider(&provider_id)?;
        Self::from_durable(
            provider_id,
            account_id,
            provider_display_name,
            account_display_name,
        )
    }

    pub(super) fn from_durable(
        provider_id: ProviderId,
        account_id: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        validate_display_name("Provider", provider_display_name.as_deref())?;
        validate_display_name("Account", account_display_name.as_deref())?;
        Ok(Self {
            provider_id,
            account_id,
            provider_display_name,
            account_display_name,
        })
    }

    #[must_use]
    pub const fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    #[must_use]
    pub const fn account_id(&self) -> &AccountId {
        &self.account_id
    }

    #[must_use]
    pub fn provider_display_name(&self) -> Option<&str> {
        self.provider_display_name.as_deref()
    }

    #[must_use]
    pub fn account_display_name(&self) -> Option<&str> {
        self.account_display_name.as_deref()
    }

    /// Stable Provider-and-Account reference using the same canonical escaping as ModelTarget.
    #[must_use]
    pub fn canonical_reference(&self) -> String {
        format!(
            "{}:{}",
            super::super::selection::encode_coordinate_segment(self.provider_id.as_str()),
            super::super::selection::encode_coordinate_segment(self.account_id.as_str()),
        )
    }
}

/// One durable stored model binding and its model presentation metadata.
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

    pub(super) fn from_durable(
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

    pub(super) fn from_durable_with_state(
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

    pub(super) fn with_last_failure(mut self, last_failure: Option<ModelLastFailure>) -> Self {
        self.last_failure = last_failure;
        self
    }

    pub(super) fn with_enabled(mut self, enabled: bool) -> Self {
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

/// Closed, secret-free classification for one actual model request failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModelRequestFailureKind {
    Authentication,
    AccessDenied,
    ModelUnavailable,
    RateLimited,
    RequestRejected,
    ProviderUnavailable,
    Transport,
    Timeout,
    Protocol,
    ResponseLimit,
    LocalConfiguration,
}

impl ModelRequestFailureKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::AccessDenied => "access_denied",
            Self::ModelUnavailable => "model_unavailable",
            Self::RateLimited => "rate_limited",
            Self::RequestRejected => "request_rejected",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::Transport => "transport",
            Self::Timeout => "timeout",
            Self::Protocol => "protocol",
            Self::ResponseLimit => "response_limit",
            Self::LocalConfiguration => "local_configuration",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "authentication" => Some(Self::Authentication),
            "access_denied" => Some(Self::AccessDenied),
            "model_unavailable" => Some(Self::ModelUnavailable),
            "rate_limited" => Some(Self::RateLimited),
            "request_rejected" => Some(Self::RequestRejected),
            "provider_unavailable" => Some(Self::ProviderUnavailable),
            "transport" => Some(Self::Transport),
            "timeout" => Some(Self::Timeout),
            "protocol" => Some(Self::Protocol),
            "response_limit" => Some(Self::ResponseLimit),
            "local_configuration" => Some(Self::LocalConfiguration),
            _ => None,
        }
    }
}

impl std::fmt::Display for ModelRequestFailureKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One warning-only per-model observation retained outside complete-binding identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelLastFailure {
    kind: ModelRequestFailureKind,
    observed_at: String,
}

impl ModelLastFailure {
    pub fn new(
        kind: ModelRequestFailureKind,
        observed_at: impl Into<String>,
    ) -> Result<Self, ModelServiceError> {
        let observed_at = observed_at.into();
        let timestamp = observed_at.parse::<jiff::Timestamp>().map_err(|_| {
            ModelServiceError::new("model last_failure observed_at must be canonical UTC RFC 3339")
        })?;
        if timestamp.subsec_nanosecond() != 0 || timestamp.to_string() != observed_at {
            return Err(ModelServiceError::new(
                "model last_failure observed_at must be canonical UTC RFC 3339 at whole-second precision",
            ));
        }
        Ok(Self { kind, observed_at })
    }

    #[must_use]
    pub const fn kind(&self) -> ModelRequestFailureKind {
        self.kind
    }

    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
}

pub(super) fn validate_state(
    accounts: &[ConnectionAccount],
    bindings: &[StoredModelBinding],
) -> Result<(), ModelServiceError> {
    let mut account_coordinates = HashSet::new();
    let mut provider_display_names = HashMap::new();
    for account in accounts {
        let coordinate = (account.provider_id().clone(), account.account_id().clone());
        if !account_coordinates.insert(coordinate.clone()) {
            return Err(ModelServiceError::new(format!(
                "duplicate stored account for Provider {} and Account {}",
                account.provider_id(),
                account.account_id()
            )));
        }
        require_consistent_provider_display(
            &mut provider_display_names,
            account.provider_id().clone(),
            account.provider_display_name(),
        )?;
    }

    let mut binding_coordinates = HashSet::new();
    for binding in bindings {
        let complete = binding.complete().binding();
        let account_coordinate = (
            complete.provider_id().clone(),
            complete.account_id().clone(),
        );
        if !account_coordinates.contains(&account_coordinate) {
            return Err(ModelServiceError::new(format!(
                "stored model for Provider {}, Account {}, Model {} has no stored account",
                complete.provider_id(),
                complete.account_id(),
                complete.model_id()
            )));
        }
        let coordinate = (
            complete.provider_id().clone(),
            complete.account_id().clone(),
            complete.model_id().clone(),
        );
        if !binding_coordinates.insert(coordinate) {
            return Err(ModelServiceError::new(format!(
                "duplicate stored model for Provider {}, Account {}, Model {}",
                complete.provider_id(),
                complete.account_id(),
                complete.model_id()
            )));
        }
    }
    Ok(())
}

pub(super) fn account_matches_binding(
    account: &ConnectionAccount,
    binding: &StoredModelBinding,
) -> bool {
    let complete = binding.complete().binding();
    account.provider_id() == complete.provider_id() && account.account_id() == complete.account_id()
}

pub(super) fn binding_matches_selection(
    binding: &StoredModelBinding,
    selection: &ModelSelection,
) -> bool {
    let complete = binding.complete().binding();
    complete.provider_id() == selection.provider()
        && complete.account_id() == selection.account()
        && complete.model_id() == selection.model()
}

fn reject_new_host_provider(provider_id: &ProviderId) -> Result<(), ModelServiceError> {
    if provider_id.as_str() == "host" {
        return Err(ModelServiceError::new(
            "new stored connections cannot use the reserved ProviderId host",
        ));
    }
    Ok(())
}

fn require_consistent_provider_display(
    names: &mut HashMap<ProviderId, Option<String>>,
    provider_id: ProviderId,
    value: Option<&str>,
) -> Result<(), ModelServiceError> {
    let value = value.map(str::to_owned);
    match names.entry(provider_id.clone()) {
        Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        },
        Entry::Occupied(entry) if entry.get() == &value => Ok(()),
        Entry::Occupied(_) => Err(ModelServiceError::new(format!(
            "inconsistent stored display name for Provider {provider_id}"
        ))),
    }
}
