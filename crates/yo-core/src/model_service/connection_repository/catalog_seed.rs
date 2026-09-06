use crate::{
    AccountId, EffectiveModelProfile, ModelServiceError, NormalizedEndpoint, ProviderId,
    VersionedProfileId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionCatalogSeed {
    provider: ProviderId,
    account: AccountId,
    provider_display_name: Option<String>,
    account_display_name: Option<String>,
    source: CatalogSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CatalogSource {
    Discovery {
        endpoint: NormalizedEndpoint,
        profile: Box<EffectiveModelProfile>,
    },
    BuiltIn {
        catalog: VersionedProfileId,
    },
}

impl ConnectionCatalogSeed {
    pub fn discovery(
        provider: ProviderId,
        account: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
        endpoint: NormalizedEndpoint,
        profile: EffectiveModelProfile,
    ) -> Result<Self, ModelServiceError> {
        // This neutral descriptor still encodes the historical openrouter_discovery
        // wire kind; reject a mismatched durable identity before it is representable.
        if provider.as_str() != "openrouter" {
            return Err(ModelServiceError::new(
                "OpenRouter discovery seed requires ProviderId openrouter",
            ));
        }
        crate::ConnectionAccount::new(
            provider.clone(),
            account.clone(),
            provider_display_name.clone(),
            account_display_name.clone(),
        )?;
        Ok(Self {
            provider,
            account,
            provider_display_name,
            account_display_name,
            source: CatalogSource::Discovery {
                endpoint,
                profile: Box::new(profile),
            },
        })
    }

    pub fn built_in(
        catalog: VersionedProfileId,
        provider: ProviderId,
        account: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
    ) -> Result<Self, ModelServiceError> {
        // These exact persisted source identities keep durable decoding closed. Service
        // endpoints, model rows, and catalog resolution belong to provider crates.
        match (provider.as_str(), catalog.as_str()) {
            ("kimi", "kimi-platform-ai/v1" | "kimi-code-membership/v1")
            | (
                "qwencloud",
                "qwencloud-coding-plan-cn/v1"
                | "qwencloud-coding-plan-intl/v1"
                | "qwencloud-token-plan-team-intl/v1",
            ) => {},
            ("kimi", _) => {
                return Err(ModelServiceError::new(format!(
                    "unsupported Kimi catalog profile {catalog}"
                )));
            },
            ("qwencloud", _) => {
                return Err(ModelServiceError::new(format!(
                    "unsupported QwenCloud catalog profile {catalog}"
                )));
            },
            _ => {
                return Err(ModelServiceError::new(format!(
                    "Provider {provider} does not own a built-in catalog"
                )));
            },
        }
        crate::ConnectionAccount::new(
            provider.clone(),
            account.clone(),
            provider_display_name.clone(),
            account_display_name.clone(),
        )?;
        Ok(Self {
            provider,
            account,
            provider_display_name,
            account_display_name,
            source: CatalogSource::BuiltIn { catalog },
        })
    }

    pub const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    pub fn provider_display_name(&self) -> Option<&str> {
        self.provider_display_name.as_deref()
    }

    pub fn account_display_name(&self) -> Option<&str> {
        self.account_display_name.as_deref()
    }

    pub(crate) const fn source(&self) -> &CatalogSource {
        &self.source
    }

    /// Returns the exact built-in adapter identity when this is a static catalog seed.
    #[must_use]
    pub fn built_in_profile(&self) -> Option<&VersionedProfileId> {
        match &self.source {
            CatalogSource::BuiltIn { catalog } => Some(catalog),
            CatalogSource::Discovery { .. } => None,
        }
    }

    /// Returns the exact endpoint and effective profile of the persisted discovery source.
    #[must_use]
    pub fn discovery_definition(&self) -> Option<(&NormalizedEndpoint, &EffectiveModelProfile)> {
        match &self.source {
            CatalogSource::Discovery { endpoint, profile } => Some((endpoint, profile.as_ref())),
            CatalogSource::BuiltIn { .. } => None,
        }
    }
}
