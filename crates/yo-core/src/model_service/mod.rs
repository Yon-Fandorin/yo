//! Provider-neutral model-service identities, bindings, catalogs, and resolved credentials.

mod binding;
mod binding_profile;
mod catalog;
mod connection_operation;
mod connection_repository;
mod credential;
mod identity;
mod kimi_catalog;
mod local_credentials;
mod openrouter_discovery;
mod profile;
mod qwencloud_catalog;
mod selection;
mod startup;

pub use binding::{ApiDialect, ConnectorId, EffectiveModelBinding, NormalizedEndpoint};
pub use binding_profile::CompleteModelBinding;
pub use catalog::{
    BindingConflict, ModelCatalog, ModelCatalogEntry, ModelCatalogProvenance, ModelContextProfile,
    ModelTokenCounter, ModelTokenCounterError,
};
pub use connection_operation::{
    ConnectionCredentialAction, ConnectionOperationError, ConnectionOperationExecutionError,
    ConnectionOperationExecutionOutcome, ConnectionOperationJournalEntry,
    ConnectionOperationJournalRepository, ConnectionOperationKind, ConnectionOperationPhase,
    ConnectionOperationRecovery, ConnectionOperationRepositoryKind, ExternalConnectionError,
    ExternalDisconnectCredentialAction, ExternalDisconnectError, LocalConnectionOperationJournal,
    LocalConnectionOperationRepositories, LocalConnectionOperationSession,
    PreparedExternalConnection, PreparedExternalDisconnect, VerifiedExternalConnection,
    plan_connection_recovery, verify_external_connection,
};
pub use connection_repository::{
    ConnectionCommit, ConnectionRepository, ConnectionRepositoryError, ConnectionRevision,
    ConnectionSnapshot, LocalConnectionOperationGuard, LocalConnectionRepository,
    ManagedConnectionAccount, ManagedConnectionBinding, PreparedConnectionMutation,
};
pub use credential::{ApiCredential, CredentialStore};
pub use identity::{AccountId, ModelId, ModelServiceError, ProviderId};
pub use kimi_catalog::{
    KimiCatalogAvailability, KimiCatalogDisabledReason, KimiCatalogError, KimiCatalogFailureKind,
    KimiCatalogModel, KimiCatalogSeed, discover_kimi_models, parse_kimi_catalog_snapshot,
};
pub use local_credentials::{
    CredentialCommit, CredentialMutationAction, CredentialRepository, CredentialRevision,
    CredentialSnapshot, LocalCredentialRepository, LocalCredentialStore, LocalCredentialStoreError,
    PreparedCredentialMutation,
};
pub use openrouter_discovery::{
    OpenRouterAuthoredModel, OpenRouterDisabledReason, OpenRouterDiscoveredModel,
    OpenRouterDiscoveryError, OpenRouterDiscoveryFailureKind, OpenRouterDiscoverySeed,
    OpenRouterModelAvailability, OpenRouterModelCapabilities, discover_openrouter_models,
};
pub use profile::{
    EffectiveModelProfile, KIMI_PRIVATE_REPLAY_PROFILE, ModelProfileLayer, ModelProfileParameters,
    SEMANTIC_REPLAY_PROFILE, VersionedProfileId,
};
pub use qwencloud_catalog::{
    QwenCloudCatalogAvailability, QwenCloudCatalogDisabledReason, QwenCloudCatalogModel,
    QwenCloudCatalogSeed,
};
pub use selection::{ModelSelection, ModelSelectionChoice, ModelSelectionController};
pub use startup::{StartupPolicy, StartupSelectionSources, StartupTarget, resolve_startup_target};

#[cfg(test)]
mod tests;
