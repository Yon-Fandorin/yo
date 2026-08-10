//! Provider-neutral model-service identities, bindings, catalogs, and resolved credentials.

mod binding;
mod catalog;
mod credential;
mod identity;
mod local_credentials;
mod selection;
mod startup;

pub use binding::{ApiDialect, ConnectorId, EffectiveModelBinding, NormalizedEndpoint};
pub use catalog::{
    ModelCatalog, ModelCatalogEntry, ModelContextProfile, ModelTokenCounter, ModelTokenCounterError,
};
pub use credential::{ApiCredential, CredentialStore};
pub use identity::{AccountId, ModelId, ModelServiceError, ProviderId};
pub use local_credentials::{LocalCredentialStore, LocalCredentialStoreError};
pub use selection::{ModelSelection, ModelSelectionChoice, ModelSelectionController};
pub use startup::{StartupPolicy, StartupSelectionSources, StartupTarget, resolve_startup_target};

#[cfg(test)]
mod tests;
