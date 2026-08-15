use std::{
    collections::{BTreeSet, HashMap},
    error::Error,
    fmt,
};

use super::{
    AccountId, ApiCredential, EffectiveModelProfile, ModelCatalogEntry, ModelId, ModelServiceError,
    NormalizedEndpoint, ProviderId,
};

mod normalize;
mod transport;

use self::normalize::normalize_catalog;
#[cfg(test)]
use self::normalize::{MAX_REMOTE_NAME_BYTES, MAX_ROWS, profile_with_remote_limits};
#[cfg(test)]
use self::transport::{
    CONNECT_TIMEOUT, DISCOVERY_TIMEOUTS, DiscoveryTimeouts, MAX_REDIRECTS, MAX_RESPONSE_BYTES,
    Origin, discovery_url, fetch_catalog_with_timeouts, is_json_media_type,
};

const OPENROUTER_PROVIDER: &str = "openrouter";

#[derive(Clone, Debug)]
pub struct OpenRouterAuthoredModel {
    entry: ModelCatalogEntry,
    input_token_limit: Option<u64>,
    max_output_tokens: Option<u64>,
}

impl OpenRouterAuthoredModel {
    pub fn new(
        entry: ModelCatalogEntry,
        input_token_limit: Option<u64>,
        max_output_tokens: Option<u64>,
    ) -> Result<Self, ModelServiceError> {
        let complete = entry.complete_binding().ok_or_else(|| {
            ModelServiceError::new(
                "OpenRouter discovery seed authored model requires a complete profile",
            )
        })?;
        if input_token_limit
            .is_some_and(|value| value != complete.profile().context().input_token_limit())
            || max_output_tokens
                .is_some_and(|value| value != complete.profile().context().max_output_tokens())
        {
            return Err(ModelServiceError::new(
                "OpenRouter authored limit provenance does not match its complete profile",
            ));
        }
        Ok(Self {
            entry,
            input_token_limit,
            max_output_tokens,
        })
    }
}

#[derive(Clone, Debug)]
pub struct OpenRouterDiscoverySeed {
    provider: ProviderId,
    account: AccountId,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
    endpoint: NormalizedEndpoint,
    base_profile: EffectiveModelProfile,
    authored_models: HashMap<ModelId, OpenRouterAuthoredModel>,
}

impl OpenRouterDiscoverySeed {
    pub fn new(
        provider: ProviderId,
        account: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
        endpoint: NormalizedEndpoint,
        base_profile: EffectiveModelProfile,
        authored_models: Vec<OpenRouterAuthoredModel>,
    ) -> Result<Self, ModelServiceError> {
        if provider.as_str() != OPENROUTER_PROVIDER {
            return Err(ModelServiceError::new(
                "OpenRouter discovery seed requires ProviderId openrouter",
            ));
        }
        super::catalog::validate_display_name("Provider", provider_display_name.as_deref())?;
        super::catalog::validate_display_name("Account", account_display_name.as_deref())?;
        let mut indexed = HashMap::new();
        for authored in authored_models {
            let binding = authored.entry.binding();
            if binding.provider_id() != &provider || binding.account_id() != &account {
                return Err(ModelServiceError::new(
                    "OpenRouter discovery seed authored model belongs to another Provider or Account",
                ));
            }
            if indexed
                .insert(binding.model_id().clone(), authored)
                .is_some()
            {
                return Err(ModelServiceError::new(
                    "OpenRouter discovery seed repeats an authored ModelId",
                ));
            }
        }
        Ok(Self {
            provider,
            account,
            provider_display_name,
            account_display_name,
            endpoint,
            base_profile,
            authored_models: indexed,
        })
    }

    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }
}

#[derive(Clone, Debug)]
pub struct OpenRouterDiscoveredModel {
    provider: ProviderId,
    account: AccountId,
    model_id: ModelId,
    entry: Option<ModelCatalogEntry>,
    display_name: String,
    capabilities: Option<OpenRouterModelCapabilities>,
    input_limit: Option<u64>,
    output_limit: Option<u64>,
    effective_tool_policy: Option<super::VersionedProfileId>,
    reasoning: Option<bool>,
    availability: OpenRouterModelAvailability,
}

impl OpenRouterDiscoveredModel {
    #[must_use]
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    #[must_use]
    pub const fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    /// Returns a routable entry only for an enabled inventory item.
    #[must_use]
    pub const fn entry(&self) -> Option<&ModelCatalogEntry> {
        self.entry.as_ref()
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub const fn capabilities(&self) -> Option<&OpenRouterModelCapabilities> {
        self.capabilities.as_ref()
    }

    #[must_use]
    pub const fn input_limit(&self) -> Option<u64> {
        self.input_limit
    }

    #[must_use]
    pub const fn output_limit(&self) -> Option<u64> {
        self.output_limit
    }

    #[must_use]
    pub const fn effective_tool_policy(&self) -> Option<&super::VersionedProfileId> {
        self.effective_tool_policy.as_ref()
    }

    #[must_use]
    pub const fn reasoning(&self) -> Option<bool> {
        self.reasoning
    }

    #[must_use]
    pub const fn availability(&self) -> OpenRouterModelAvailability {
        self.availability
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        matches!(self.availability, OpenRouterModelAvailability::Enabled)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRouterModelCapabilities {
    input_modalities: BTreeSet<String>,
    output_modalities: BTreeSet<String>,
    supported_parameters: BTreeSet<String>,
}

impl OpenRouterModelCapabilities {
    #[must_use]
    pub const fn input_modalities(&self) -> &BTreeSet<String> {
        &self.input_modalities
    }

    #[must_use]
    pub const fn output_modalities(&self) -> &BTreeSet<String> {
        &self.output_modalities
    }

    #[must_use]
    pub const fn supported_parameters(&self) -> &BTreeSet<String> {
        &self.supported_parameters
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenRouterModelAvailability {
    Enabled,
    Disabled(OpenRouterDisabledReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenRouterDisabledReason {
    CapabilitiesUnavailable,
    TextInputUnsupported,
    TextOutputUnsupported,
    ToolPolicyUnsupported,
    ProfileUnavailable,
}

impl OpenRouterDisabledReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CapabilitiesUnavailable => "capabilities unavailable",
            Self::TextInputUnsupported => "text input unsupported",
            Self::TextOutputUnsupported => "text output unsupported",
            Self::ToolPolicyUnsupported => "tool policy unsupported",
            Self::ProfileUnavailable => "profile unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenRouterDiscoveryFailureKind {
    Configuration,
    Transport,
    HttpStatus,
    MediaType,
    Limit,
    Protocol,
    Timeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenRouterDiscoveryError {
    kind: OpenRouterDiscoveryFailureKind,
    message: String,
}

impl OpenRouterDiscoveryError {
    fn new(kind: OpenRouterDiscoveryFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> OpenRouterDiscoveryFailureKind {
        self.kind
    }
}

impl fmt::Display for OpenRouterDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl Error for OpenRouterDiscoveryError {}

pub fn discover_openrouter_models(
    seed: &OpenRouterDiscoverySeed,
    credential: &ApiCredential,
) -> Result<Vec<OpenRouterDiscoveredModel>, OpenRouterDiscoveryError> {
    let bytes = transport::fetch_catalog(&seed.endpoint, credential)?;
    normalize_catalog(seed, &bytes)
}

fn failure(
    kind: OpenRouterDiscoveryFailureKind,
    message: impl Into<String>,
) -> OpenRouterDiscoveryError {
    OpenRouterDiscoveryError::new(kind, message)
}

fn limit_failure(message: impl Into<String>) -> OpenRouterDiscoveryError {
    failure(OpenRouterDiscoveryFailureKind::Limit, message)
}

fn timeout_failure(message: impl Into<String>) -> OpenRouterDiscoveryError {
    failure(OpenRouterDiscoveryFailureKind::Timeout, message)
}

#[cfg(test)]
mod tests;
