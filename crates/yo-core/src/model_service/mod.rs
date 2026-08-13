//! Provider-neutral model-service identities, bindings, catalogs, and resolved credentials.

mod binding;
mod binding_profile;
mod catalog;
mod connection_operation;
mod connection_repository;
mod credential;
mod identity;
mod local_credentials;
mod profile;
mod selection;
mod startup;

pub use binding::{ApiDialect, ConnectorId, EffectiveModelBinding, NormalizedEndpoint};
pub use binding_profile::CompleteModelBinding;
pub use catalog::{
    ModelCatalog, ModelCatalogEntry, ModelContextProfile, ModelTokenCounter, ModelTokenCounterError,
};
pub use connection_operation::{
    ConnectionCredentialAction, ConnectionOperationError, ConnectionOperationExecutionError,
    ConnectionOperationExecutionOutcome, ConnectionOperationJournalEntry,
    ConnectionOperationJournalRepository, ConnectionOperationKind, ConnectionOperationPhase,
    ConnectionOperationRecovery, ConnectionOperationRepositoryKind,
    LocalConnectionOperationJournal, LocalConnectionOperationRepositories,
    LocalConnectionOperationSession, plan_connection_recovery,
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
pub use profile::{
    EffectiveModelProfile, ModelProfileLayer, ModelProfileParameters, VersionedProfileId,
};
pub use selection::{ModelSelection, ModelSelectionChoice, ModelSelectionController};
pub use startup::{StartupPolicy, StartupSelectionSources, StartupTarget, resolve_startup_target};

#[cfg(test)]
mod tests;
