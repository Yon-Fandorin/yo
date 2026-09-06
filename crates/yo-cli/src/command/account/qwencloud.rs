use yo_core::{AccountCapacitySnapshot, AccountId, ApiCredential, ProviderId};
pub(super) use yo_provider_qwencloud::QwenCloudProviderData;

use crate::AppError;

pub(super) struct QwenCloudCapacityError {
    source: AppError,
    expired_session: bool,
}

impl QwenCloudCapacityError {
    pub(super) fn is_expired_session(&self) -> bool {
        self.expired_session
    }

    pub(super) fn into_app_error(self) -> AppError {
        self.source
    }
}

/// Validates provider-owned browser-session material before the CLI persists it.
pub(super) fn validate_account_session(cookie: ApiCredential) -> Result<ApiCredential, AppError> {
    yo_provider_qwencloud::validate_account_session(cookie)
        .map_err(|error| AppError::message(error.to_string()))
}

/// Reads capacity through the provider-owned fixed-origin console protocol.
pub(super) fn read_account_capacity(
    provider: &ProviderId,
    account: &AccountId,
    cookie: &ApiCredential,
) -> Result<(AccountCapacitySnapshot, QwenCloudProviderData), QwenCloudCapacityError> {
    yo_provider_qwencloud::read_account_capacity(provider, account, cookie).map_err(|error| {
        QwenCloudCapacityError {
            expired_session: error.is_expired_session(),
            source: AppError::message(error.to_string()),
        }
    })
}

#[cfg(test)]
pub(super) fn fixture_capacity_error(expired_session: bool) -> QwenCloudCapacityError {
    QwenCloudCapacityError {
        source: AppError::message(if expired_session {
            "fixture QwenCloud console session expired"
        } else {
            "fixture QwenCloud capacity failure"
        }),
        expired_session,
    }
}
