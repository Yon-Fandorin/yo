use crate::{
    AccountId, EffectiveModelProfile, KimiCatalogSeed, ModelServiceError, NormalizedEndpoint,
    OpenRouterDiscoverySeed, ProviderId, QwenCloudCatalogSeed, VersionedProfileId,
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
    OpenRouter {
        endpoint: NormalizedEndpoint,
        profile: Box<EffectiveModelProfile>,
    },
    BuiltIn {
        catalog: VersionedProfileId,
    },
}

impl ConnectionCatalogSeed {
    pub fn openrouter(
        provider: ProviderId,
        account: AccountId,
        provider_display_name: Option<String>,
        account_display_name: Option<String>,
        endpoint: NormalizedEndpoint,
        profile: EffectiveModelProfile,
    ) -> Result<Self, ModelServiceError> {
        OpenRouterDiscoverySeed::new(
            provider.clone(),
            account.clone(),
            provider_display_name.clone(),
            account_display_name.clone(),
            endpoint.clone(),
            profile.clone(),
            Vec::new(),
        )?;
        Ok(Self {
            provider,
            account,
            provider_display_name,
            account_display_name,
            source: CatalogSource::OpenRouter {
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
        match provider.as_str() {
            "qwencloud" => {
                QwenCloudCatalogSeed::resolve(
                    catalog.clone(),
                    provider.clone(),
                    account.clone(),
                    provider_display_name.clone(),
                    account_display_name.clone(),
                )?;
            },
            "kimi" => {
                KimiCatalogSeed::resolve(
                    catalog.clone(),
                    provider.clone(),
                    account.clone(),
                    provider_display_name.clone(),
                    account_display_name.clone(),
                )?;
            },
            _ => {
                return Err(ModelServiceError::new(format!(
                    "Provider {provider} does not own a built-in catalog"
                )));
            },
        }
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

    pub(crate) const fn source(&self) -> &CatalogSource {
        &self.source
    }

    /// Returns the exact built-in adapter identity when this is a static catalog seed.
    #[must_use]
    pub fn built_in_profile(&self) -> Option<&VersionedProfileId> {
        match &self.source {
            CatalogSource::BuiltIn { catalog } => Some(catalog),
            CatalogSource::OpenRouter { .. } => None,
        }
    }

    /// Returns the exact endpoint and effective profile when this is an OpenRouter seed.
    #[must_use]
    pub fn openrouter_definition(&self) -> Option<(&NormalizedEndpoint, &EffectiveModelProfile)> {
        match &self.source {
            CatalogSource::OpenRouter { endpoint, profile } => Some((endpoint, profile.as_ref())),
            CatalogSource::BuiltIn { .. } => None,
        }
    }

    pub fn openrouter_seed(&self) -> Result<Option<OpenRouterDiscoverySeed>, ModelServiceError> {
        let CatalogSource::OpenRouter { endpoint, profile } = &self.source else {
            return Ok(None);
        };
        OpenRouterDiscoverySeed::new(
            self.provider.clone(),
            self.account.clone(),
            self.provider_display_name.clone(),
            self.account_display_name.clone(),
            endpoint.clone(),
            profile.as_ref().clone(),
            Vec::new(),
        )
        .map(Some)
    }

    pub fn qwencloud_seed(&self) -> Result<Option<QwenCloudCatalogSeed>, ModelServiceError> {
        let CatalogSource::BuiltIn { catalog } = &self.source else {
            return Ok(None);
        };
        if self.provider.as_str() != "qwencloud" {
            return Ok(None);
        }
        QwenCloudCatalogSeed::resolve(
            catalog.clone(),
            self.provider.clone(),
            self.account.clone(),
            self.provider_display_name.clone(),
            self.account_display_name.clone(),
        )
        .map(Some)
    }

    pub fn kimi_seed(&self) -> Result<Option<KimiCatalogSeed>, ModelServiceError> {
        let CatalogSource::BuiltIn { catalog } = &self.source else {
            return Ok(None);
        };
        if self.provider.as_str() != "kimi" {
            return Ok(None);
        }
        KimiCatalogSeed::resolve(
            catalog.clone(),
            self.provider.clone(),
            self.account.clone(),
            self.provider_display_name.clone(),
            self.account_display_name.clone(),
        )
        .map(Some)
    }
}
