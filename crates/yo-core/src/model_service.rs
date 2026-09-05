//! Provider-neutral model-service identities, bindings, catalogs, and resolved credentials.

mod account_capacity;
mod binding;
mod binding_profile;
mod catalog;
mod connection_observation;
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

pub use account_capacity::{
    AccountCapacityBucket, AccountCapacitySnapshot, AccountCapacityWindow, AccountCredits,
};
pub use binding::{ApiDialect, ConnectorId, EffectiveModelBinding, NormalizedEndpoint};
pub use binding_profile::CompleteModelBinding;
pub use catalog::{
    ModelCatalog, ModelCatalogEntry, ModelContextProfile, ModelTokenCounter, ModelTokenCounterError,
};
pub use connection_observation::{
    LocalModelRequestObservation, ModelObservationWriteOutcome, ModelRequestOutcome,
};
pub use connection_operation::{
    ConnectionCredentialAction, ConnectionOperationError, ConnectionOperationExecutionError,
    ConnectionOperationExecutionOutcome, ConnectionOperationJournalEntry,
    ConnectionOperationJournalRepository, ConnectionOperationKind, ConnectionOperationPhase,
    ConnectionOperationRecovery, ConnectionOperationRepositoryKind, ExternalConnectionError,
    ExternalDisconnectCredentialAction, ExternalDisconnectError, LocalConnectionOperationJournal,
    LocalConnectionOperationRepositories, LocalConnectionOperationSession,
    PreparedExternalConnection, PreparedExternalDisconnect, plan_connection_recovery,
};
pub use connection_repository::{
    ConnectionAccount, ConnectionCatalogSeed, ConnectionCommit, ConnectionRepository,
    ConnectionRepositoryError, ConnectionRevision, ConnectionSnapshot,
    LocalConnectionOperationGuard, LocalConnectionRepository, ModelLastFailure,
    ModelRequestFailureKind, PreparedConnectionMutation, StoredModelBinding,
};
pub use credential::{ApiCredential, CredentialStore};
pub use identity::{AccountId, ModelId, ModelServiceError, ModelServiceErrorKind, ProviderId};
pub use kimi_catalog::{
    KimiAccountCapacityError, KimiAccountCapacityFailureKind, KimiCatalogAvailability,
    KimiCatalogDisabledReason, KimiCatalogError, KimiCatalogFailureKind, KimiCatalogModel,
    KimiCatalogSeed, discover_kimi_models, parse_kimi_account_capacity_snapshot,
    parse_kimi_catalog_snapshot, read_kimi_account_capacity,
};
pub use local_credentials::{
    CredentialCommit, CredentialMutationAction, CredentialRepository, CredentialRevision,
    CredentialSnapshot, LocalCredentialRepository, LocalCredentialStore, LocalCredentialStoreError,
    PreparedAccountSessionMutation, PreparedCredentialMutation,
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
pub use selection::{
    HostCatalogModel, HostModelCatalog, HostModelSelection, ModelPickerChoice, ModelPickerSection,
    ModelPickerTarget, ModelSelection, ModelSelectionChoice, ModelSelectionController,
    derive_host_account_id, derive_host_catalog_revision,
};
pub use startup::{
    HostId, StartupPolicy, StartupSelectionSources, StartupTarget, resolve_startup_target,
};

#[cfg(test)]
mod tests;
