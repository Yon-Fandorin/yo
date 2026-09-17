use std::collections::HashSet;

use super::{super::error::ConnectionRepositoryError, account::ConnectionAccount};
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
        // 이 중립 descriptor는 historical openrouter_discovery
        // wire kind를 계속 인코딩하므로, 표현 가능한 값이 되기 전에 불일치하는 durable identity를
        // 거절합니다.
        if provider.as_str() != "openrouter" {
            return Err(ModelServiceError::new(
                "OpenRouter discovery seed requires ProviderId openrouter",
            ));
        }
        ConnectionAccount::new(
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
        // 이 정확한 persisted source identity들이 durable decoding을 닫힌 집합으로 유지합니다.
        // Service endpoint, model row, catalog resolution은 provider crate의 책임입니다.
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
        ConnectionAccount::new(
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

    /// static catalog seed일 때 정확한 built-in adapter identity를 반환합니다.
    #[must_use]
    pub fn built_in_profile(&self) -> Option<&VersionedProfileId> {
        match &self.source {
            CatalogSource::BuiltIn { catalog } => Some(catalog),
            CatalogSource::Discovery { .. } => None,
        }
    }

    /// persisted discovery source의 정확한 endpoint와 effective profile을 반환합니다.
    #[must_use]
    pub fn discovery_definition(&self) -> Option<(&NormalizedEndpoint, &EffectiveModelProfile)> {
        match &self.source {
            CatalogSource::Discovery { endpoint, profile } => Some((endpoint, profile.as_ref())),
            CatalogSource::BuiltIn { .. } => None,
        }
    }
}

pub(in super::super::super) fn validate_catalog_seeds(
    accounts: &[ConnectionAccount],
    seeds: &[ConnectionCatalogSeed],
) -> Result<(), ConnectionRepositoryError> {
    let mut coordinates = HashSet::new();
    for seed in seeds {
        if !coordinates.insert((seed.provider().clone(), seed.account().clone()))
            || !accounts.iter().any(|account| {
                account.provider_id() == seed.provider() && account.account_id() == seed.account()
            })
        {
            return Err(ConnectionRepositoryError::InvalidMutation);
        }
    }
    Ok(())
}
