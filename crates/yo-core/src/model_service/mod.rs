//! Provider-neutral model-service identities, bindings, catalogs, and resolved credentials.

mod binding;
mod catalog;
mod connection_operation;
mod connection_repository;
mod credential;
mod identity;
mod local_credentials;
mod selection;
mod startup;

pub use binding::{ApiDialect, ConnectorId, EffectiveModelBinding, NormalizedEndpoint};
pub use catalog::{
    ModelCatalog, ModelCatalogEntry, ModelContextProfile, ModelTokenCounter, ModelTokenCounterError,
};
pub use connection_operation::{
    ConnectionCredentialAction, ConnectionOperationError, ConnectionOperationJournalEntry,
    ConnectionOperationJournalRepository, ConnectionOperationKind, ConnectionOperationPhase,
    ConnectionOperationRecovery, LocalConnectionOperationJournal, plan_connection_recovery,
};
pub use connection_repository::{
    ConnectionCommit, ConnectionRepository, ConnectionRepositoryError, ConnectionRevision,
    ConnectionSnapshot, LocalConnectionOperationGuard, LocalConnectionRepository,
    PreparedConnectionMutation,
};
pub use credential::{ApiCredential, CredentialStore};
pub use identity::{AccountId, ModelId, ModelServiceError, ProviderId};
pub use local_credentials::{
    CredentialCommit, CredentialMutationAction, CredentialRepository, CredentialRevision,
    CredentialSnapshot, LocalCredentialRepository, LocalCredentialStore, LocalCredentialStoreError,
    PreparedCredentialMutation,
};
pub use selection::{ModelSelection, ModelSelectionChoice, ModelSelectionController};
pub use startup::{StartupPolicy, StartupSelectionSources, StartupTarget, resolve_startup_target};

#[cfg(test)]
mod tests;
