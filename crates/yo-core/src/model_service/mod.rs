//! Provider-neutral model-service identities, bindings, catalogs, and resolved credentials.

mod binding;
mod catalog;
mod credential;
mod identity;
mod local_credentials;

pub use binding::{ApiProtocol, ConnectorId, EffectiveModelBinding, NormalizedEndpoint};
pub use catalog::{ModelCatalog, ModelCatalogEntry};
pub use credential::{ApiCredential, CredentialStore};
pub use identity::{AccountId, ModelId, ModelServiceError, ProviderId};
pub use local_credentials::{LocalCredentialStore, LocalCredentialStoreError};

#[cfg(test)]
mod tests;
