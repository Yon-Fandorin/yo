use std::{collections::HashMap, error::Error, fmt};

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
    entry: ModelCatalogEntry,
    display_name: String,
    reasoning: bool,
}

impl OpenRouterDiscoveredModel {
    #[must_use]
    pub const fn entry(&self) -> &ModelCatalogEntry {
        &self.entry
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub const fn reasoning(&self) -> bool {
        self.reasoning
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
