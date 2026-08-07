use std::{collections::HashMap, fmt};

use super::{AccountId, ModelServiceError};

const MAX_API_CREDENTIAL_BYTES: usize = 16 * 1024;

#[derive(Clone, Eq, PartialEq)]
pub struct ApiCredential(Box<str>);

impl ApiCredential {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelServiceError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_API_CREDENTIAL_BYTES {
            return Err(ModelServiceError::new(format!(
                "API credential must contain 1 to {MAX_API_CREDENTIAL_BYTES} bytes"
            )));
        }
        if value.chars().any(char::is_control) {
            return Err(ModelServiceError::new(
                "API credential must not contain control characters",
            ));
        }
        Ok(Self(value.into_boxed_str()))
    }

    /// Returns the original value only for constructing an authenticated request.
    ///
    /// Callers must not place this value in diagnostics, persistence, or frontend events.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiCredential([REDACTED])")
    }
}

impl fmt::Display for ApiCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Default)]
pub struct CredentialStore {
    accounts: HashMap<AccountId, ApiCredential>,
}

impl CredentialStore {
    pub fn new(
        accounts: impl IntoIterator<Item = (AccountId, ApiCredential)>,
    ) -> Result<Self, ModelServiceError> {
        let mut store = Self::default();
        for (account_id, credential) in accounts {
            if store
                .accounts
                .insert(account_id.clone(), credential)
                .is_some()
            {
                return Err(ModelServiceError::new(format!(
                    "duplicate credential for AccountId {account_id}"
                )));
            }
        }
        Ok(store)
    }

    #[must_use]
    pub fn resolve(&self, account_id: &AccountId) -> Option<&ApiCredential> {
        self.accounts.get(account_id)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.accounts.len()
    }
}

impl fmt::Debug for CredentialStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialStore")
            .field("account_count", &self.accounts.len())
            .finish()
    }
}
