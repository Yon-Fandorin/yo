use std::{error::Error, fmt};

use super::{
    AccountId, ApiCredential, ModelCatalogEntry, ModelId, ModelServiceError, NormalizedEndpoint,
    ProviderId, VersionedProfileId,
};

mod normalize;
mod transport;

use self::normalize::normalize_catalog;

const KIMI_PROVIDER: &str = "kimi";
const KIMI_PLATFORM_CATALOG_PROFILE: &str = "kimi-platform-ai/v1";
const KIMI_CODE_CATALOG_PROFILE: &str = "kimi-code-membership/v1";
const KIMI_PLATFORM_ENDPOINT: &str = "https://api.moonshot.ai/v1/";
const KIMI_CODE_ENDPOINT: &str = "https://api.kimi.com/coding/v1/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KimiCatalogProduct {
    Platform,
    CodeMembership,
}

#[derive(Clone, Debug)]
pub struct KimiCatalogSeed {
    profile: VersionedProfileId,
    provider: ProviderId,
    account: AccountId,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
    endpoint: NormalizedEndpoint,
    product: KimiCatalogProduct,
}

impl KimiCatalogSeed {
    pub fn resolve(
        profile: VersionedProfileId,
        provider: ProviderId,
        account: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        if provider.as_str() != KIMI_PROVIDER {
            return Err(ModelServiceError::new(format!(
                "catalog profile {profile} requires ProviderId {KIMI_PROVIDER}"
            )));
        }
        let (product, endpoint) = match profile.as_str() {
            KIMI_PLATFORM_CATALOG_PROFILE => (KimiCatalogProduct::Platform, KIMI_PLATFORM_ENDPOINT),
            KIMI_CODE_CATALOG_PROFILE => (KimiCatalogProduct::CodeMembership, KIMI_CODE_ENDPOINT),
            _ => {
                return Err(ModelServiceError::new(format!(
                    "unsupported Kimi catalog profile {profile}"
                )));
            },
        };
        super::catalog::validate_display_name("Provider", provider_display_name.as_deref())?;
        super::catalog::validate_display_name("Account", account_display_name.as_deref())?;
        Ok(Self {
            profile,
            provider,
            account,
            provider_display_name: provider_display_name.or_else(|| Some("Kimi".to_owned())),
            account_display_name,
            endpoint: NormalizedEndpoint::parse(endpoint)?,
            product,
        })
    }

    pub const fn profile(&self) -> &VersionedProfileId {
        &self.profile
    }

    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub const fn account(&self) -> &AccountId {
        &self.account
    }
}

#[derive(Clone, Debug)]
pub struct KimiCatalogModel {
    provider: ProviderId,
    account: AccountId,
    model_id: ModelId,
    entry: Option<ModelCatalogEntry>,
    input_limit: Option<u64>,
    output_limit: Option<u64>,
    reasoning: Option<bool>,
    recommended: bool,
    high_speed: bool,
    availability: KimiCatalogAvailability,
}

impl KimiCatalogModel {
    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    pub const fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    pub fn display_name(&self) -> &str {
        self.model_id.as_str()
    }

    pub const fn entry(&self) -> Option<&ModelCatalogEntry> {
        self.entry.as_ref()
    }

    pub const fn input_limit(&self) -> Option<u64> {
        self.input_limit
    }

    pub const fn output_limit(&self) -> Option<u64> {
        self.output_limit
    }

    pub const fn reasoning(&self) -> Option<bool> {
        self.reasoning
    }

    pub const fn recommended(&self) -> bool {
        self.recommended
    }

    pub const fn high_speed(&self) -> bool {
        self.high_speed
    }

    pub const fn availability(&self) -> KimiCatalogAvailability {
        self.availability
    }

    pub const fn is_enabled(&self) -> bool {
        matches!(self.availability, KimiCatalogAvailability::Enabled)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KimiCatalogAvailability {
    Enabled,
    Disabled(KimiCatalogDisabledReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KimiCatalogDisabledReason {
    CapabilityConflict,
    ProfileUnavailable,
    ProviderRetirement,
}

impl KimiCatalogDisabledReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CapabilityConflict => "capability conflict",
            Self::ProfileUnavailable => "profile unavailable",
            Self::ProviderRetirement => "provider retirement",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KimiCatalogFailureKind {
    Configuration,
    Transport,
    HttpStatus,
    MediaType,
    Limit,
    Protocol,
    Timeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KimiCatalogError {
    kind: KimiCatalogFailureKind,
    message: String,
}

impl KimiCatalogError {
    fn new(kind: KimiCatalogFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub const fn kind(&self) -> KimiCatalogFailureKind {
        self.kind
    }
}

impl fmt::Display for KimiCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl Error for KimiCatalogError {}

pub fn discover_kimi_models(
    seed: &KimiCatalogSeed,
    credential: &ApiCredential,
) -> Result<Vec<KimiCatalogModel>, KimiCatalogError> {
    let bytes = transport::fetch_catalog(&seed.endpoint, credential)?;
    parse_kimi_catalog_snapshot(seed, &bytes)
}

/// Decodes one bounded authenticated `/models` response for the supplied account seed.
pub fn parse_kimi_catalog_snapshot(
    seed: &KimiCatalogSeed,
    bytes: &[u8],
) -> Result<Vec<KimiCatalogModel>, KimiCatalogError> {
    normalize_catalog(seed, bytes)
}

fn failure(kind: KimiCatalogFailureKind, message: impl Into<String>) -> KimiCatalogError {
    KimiCatalogError::new(kind, message)
}

fn limit_failure(message: impl Into<String>) -> KimiCatalogError {
    failure(KimiCatalogFailureKind::Limit, message)
}

fn timeout_failure(message: impl Into<String>) -> KimiCatalogError {
    failure(KimiCatalogFailureKind::Timeout, message)
}

#[cfg(test)]
mod tests;
