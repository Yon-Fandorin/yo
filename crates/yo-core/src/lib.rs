//! Frontend-independent agent execution semantics for yo.

mod agent_session;
mod backend;
mod command;
mod engine;
mod event;
mod host;
mod input;
mod journal;
mod model_connector;
mod model_profile_admission;
mod model_service;
mod readiness;
mod request_trace;
mod runtime;
mod session;
pub mod session_repository;
mod skill_reference;
mod tool;
mod workspace_reference;

pub use agent_session::{
    AgentIntent, AgentSession, AgentSessionError, AgentSessionPoll, BackendReplacementOutcome,
    CommandAdmission, PendingCommand,
};
pub(crate) use backend::ProviderPrivateReplayPayload;
pub use backend::{
    AgentBackend, BackendAdapter, BackendBindingEvidence, BackendCapabilities,
    BackendCommandEvidence, BackendEvent, BackendFailure, BackendFailureKind, BackendIdentity,
    BackendOutcomeEvidence, BackendPoll, BackendRequestEvidence, BackendResumeTarget,
    BackendScriptStep, BackendStopHandle, ContinuationStrategy, ModelReplay, ModelReplayBudget,
    ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole, ModelReplayTool,
    ProviderPrivateReplayEnvelope, ReplayExecutor, ReplayProfile, ScriptedBackend,
    provider_private_schema,
};
pub use command::{ActivityResponse, AgentCommand, ApprovalDecision};
pub use engine::{AgentEngine, AgentRejection, ExpectedResponse, ResponseKind};
pub use event::{ActivityKind, ActivityOutcome, ActivityUpdate, AgentEvent, Failure, TurnOutcome};
pub use host::{
    HostWorkspacePath, HostWorkspacePathError, LocalWorkspaceHostIdentity,
    LocalWorkspaceHostIdentityError, WorkspaceHostId, WorkspaceHostIdError,
    WorkspaceHostIdGenerationError,
};
pub use input::{
    InputReference, InputSubmission, SubmissionId, SubmissionIdError, SubmissionIdGenerationError,
    SubmissionOutcome, SubmissionRejection, SubmissionRejectionKind, UserInput, UserInputError,
    skill_reference_projection, workspace_reference_projection,
};
pub use journal::{
    DurabilityGapCause, JournalDurability, JournalSequence, RequestTraceReader, RequestTraceSlice,
    TranscriptEntry, TranscriptObservation, TranscriptObservationEntry,
    TranscriptObservationSequence, TranscriptObservationSlice, TranscriptReader, TranscriptRecord,
    TranscriptSlice,
};
pub use model_connector::{
    CacheReadInputTokens, ConnectorError, ConnectorFailureKind, FunctionTool,
    ModelCacheAffinityHint, ModelConnector, ModelConnectorCancellation, ModelConnectorEvent,
    ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorLimits, ModelConnectorPoll,
    ModelConnectorRequest, ModelConnectorStreamPort, ModelConnectorTerminal, ModelConnectorUsage,
    ReasoningChannel, ReasoningEffort, RequestToolExposure, ResponseTerminal,
    ResponsesCancellation, ResponsesConnectorLimits, ResponsesEvent, ResponsesInputItem,
    ResponsesInputRole, ResponsesPoll, ResponsesRequest, ResponsesUsage,
};
#[doc(hidden)]
pub use model_profile_admission::{
    AdmittedCompleteBinding, AdmittedModelProfile, AdmittedReplayProfile, AdmittedToolPolicy,
    admit_new_complete_binding,
};
pub use model_service::{
    AccountId, ApiCredential, ApiDialect, CompleteModelBinding, ConnectionAccount,
    ConnectionCatalogSeed, ConnectionCommit, ConnectionCredentialAction, ConnectionOperationError,
    ConnectionOperationExecutionError, ConnectionOperationExecutionOutcome,
    ConnectionOperationJournalEntry, ConnectionOperationJournalRepository, ConnectionOperationKind,
    ConnectionOperationPhase, ConnectionOperationRecovery, ConnectionOperationRepositoryKind,
    ConnectionRepository, ConnectionRepositoryError, ConnectionRevision, ConnectionSnapshot,
    ConnectorId, CredentialCommit, CredentialMutationAction, CredentialRepository,
    CredentialRevision, CredentialSnapshot, CredentialStore, EffectiveModelBinding,
    EffectiveModelProfile, ExternalConnectionError, ExternalDisconnectCredentialAction,
    ExternalDisconnectError, HostId, KIMI_PRIVATE_REPLAY_PROFILE, KimiCatalogAvailability,
    KimiCatalogDisabledReason, KimiCatalogError, KimiCatalogFailureKind, KimiCatalogModel,
    KimiCatalogSeed, LocalConnectionOperationGuard, LocalConnectionOperationJournal,
    LocalConnectionOperationRepositories, LocalConnectionOperationSession,
    LocalConnectionRepository, LocalCredentialRepository, LocalCredentialStore,
    LocalCredentialStoreError, LocalModelRequestObservation, ModelCatalog, ModelCatalogEntry,
    ModelContextProfile, ModelId, ModelLastFailure, ModelObservationWriteOutcome,
    ModelProfileLayer, ModelProfileParameters, ModelRequestFailureKind, ModelRequestOutcome,
    ModelSelection, ModelSelectionChoice, ModelSelectionController, ModelServiceError,
    ModelTokenCounter, ModelTokenCounterError, NormalizedEndpoint, OpenRouterAuthoredModel,
    OpenRouterDisabledReason, OpenRouterDiscoveredModel, OpenRouterDiscoveryError,
    OpenRouterDiscoveryFailureKind, OpenRouterDiscoverySeed, OpenRouterModelAvailability,
    OpenRouterModelCapabilities, PreparedConnectionMutation, PreparedCredentialMutation,
    PreparedExternalConnection, PreparedExternalDisconnect, ProviderId,
    QwenCloudCatalogAvailability, QwenCloudCatalogDisabledReason, QwenCloudCatalogModel,
    QwenCloudCatalogSeed, SEMANTIC_REPLAY_PROFILE, StartupPolicy, StartupSelectionSources,
    StartupTarget, StoredModelBinding, VersionedProfileId, discover_kimi_models,
    discover_openrouter_models, parse_kimi_catalog_snapshot, plan_connection_recovery,
    resolve_startup_target,
};
pub use request_trace::{RequestTraceEntry, RequestTraceRecord};
pub use runtime::{AgentRuntime, RuntimeError, RuntimePoll};
pub use session::{
    ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionDescriptor, SessionId,
    SessionIdError, SessionIdGenerationError, SessionStartTime, TurnId, TurnRef,
};
pub use session_repository::{
    CODEX_USAGE_SCHEMA, CacheReadShare, CacheReadSummary, GROK_USAGE_SCHEMA, MANAGED_USAGE_SCHEMA,
    SessionUsage, SessionUsageAggregates, SessionUsageError, SessionUsageProjection,
    SessionUsageProvider, SessionUsageReceipt, SessionUsageSource, UsageAggregate, UsageCoverage,
    UsageValue,
};
pub use skill_reference::{
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceProvider,
    SkillReferenceProviderPoll, SkillReferenceScope, SkillReferenceSearchRequest,
    SkillReferenceSearchStatus, SkillReferenceSearchUpdate, search_skill_reference_candidates,
};
pub use tool::{
    FrozenToolRegistry, TOOL_SCHEMA_DIALECT, ToolApprovalBinding, ToolApprovalRequirement,
    ToolDefinition, ToolEffect, ToolExecution, ToolExecutionError, ToolExecutionHost,
    ToolExecutionOutcome, ToolExecutionPoll, ToolExecutionRequest, ToolExecutionResult, ToolId,
    ToolRegistry, ToolRegistryError, ToolSemanticAdmission, ToolSemanticAdmissionError,
    ToolValidationError, ToolValidationFailure, ValidatedToolCall,
};
pub use workspace_reference::{
    LocalWorkspaceReferenceProvider, WorkspaceReference, WorkspaceReferenceCandidate,
    WorkspaceReferenceKind, WorkspaceReferenceProvider, WorkspaceReferenceProviderPoll,
    WorkspaceReferenceSearchRequest, WorkspaceReferenceSearchStatus,
    WorkspaceReferenceSearchUpdate, normalized_search_key,
};

#[cfg(test)]
pub(crate) fn fixture_session(value: u64) -> SessionId {
    let uuid = uuid::Uuid::from_u128(0x0189_0f00_0000_7000_8000_0000_0000_0000 | u128::from(value));
    SessionId::from_uuid(uuid).expect("the test Session fixture is a UUIDv7")
}

#[cfg(test)]
pub(crate) fn fixture_descriptor(session_id: SessionId) -> SessionDescriptor {
    SessionDescriptor::for_session(
        session_id,
        "10000000-0000-4000-8000-000000000001"
            .parse()
            .expect("the test Host fixture is a UUIDv4"),
        HostWorkspacePath::from_unix_bytes(b"/workspace".to_vec())
            .expect("the test workspace fixture is absolute"),
    )
}

#[cfg(test)]
mod tests;
