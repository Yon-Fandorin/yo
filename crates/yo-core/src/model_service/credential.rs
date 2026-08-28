use std::{collections::HashMap, fmt};

use super::{AccountId, ModelServiceError, ProviderId};

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
    credentials: HashMap<(ProviderId, AccountId), ApiCredential>,
    auxiliary_secret_material: Vec<ApiCredential>,
}

impl CredentialStore {
    pub fn new(
        credentials: impl IntoIterator<Item = ((ProviderId, AccountId), ApiCredential)>,
    ) -> Result<Self, ModelServiceError> {
        let mut store = Self::default();
        for ((provider_id, account_id), credential) in credentials {
            if store
                .credentials
                .insert((provider_id.clone(), account_id.clone()), credential)
                .is_some()
            {
                return Err(ModelServiceError::new(format!(
                    "duplicate credential for ProviderId {provider_id} and AccountId {account_id}"
                )));
            }
        }
        Ok(store)
    }

    #[must_use]
    pub fn resolve(
        &self,
        provider_id: &ProviderId,
        account_id: &AccountId,
    ) -> Option<&ApiCredential> {
        self.credentials
            .get(&(provider_id.clone(), account_id.clone()))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.credentials.len()
    }

    pub(super) fn retain_auxiliary_secret_material(
        &mut self,
        credentials: impl IntoIterator<Item = ApiCredential>,
    ) {
        self.auxiliary_secret_material.extend(credentials);
    }

    /// Reports whether a semantic value contains any credential in this snapshot.
    ///
    /// This permits redaction gates to cover every configured Account without exposing an
    /// iterator over the underlying secret values.
    #[must_use]
    pub fn contains_secret_material(&self, value: &str) -> bool {
        self.credentials
            .values()
            .chain(self.auxiliary_secret_material.iter())
            .any(|credential| value.contains(credential.expose_secret()))
    }
}

impl fmt::Debug for CredentialStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialStore")
            .field("credential_count", &self.credentials.len())
            .finish()
    }
}
