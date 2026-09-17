use super::{super::super::catalog::validate_display_name, reject_new_host_provider};
use crate::{AccountId, ModelServiceError, ProviderId};

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

    pub(in super::super) fn from_durable(
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

    /// ModelTarget과 동일한 canonical escaping을 사용하는 안정적인 Provider-and-Account 참조입니다.
    #[must_use]
    pub fn canonical_reference(&self) -> String {
        format!(
            "{}:{}",
            super::super::super::selection::encode_coordinate_segment(self.provider_id.as_str()),
            super::super::super::selection::encode_coordinate_segment(self.account_id.as_str()),
        )
    }
}
