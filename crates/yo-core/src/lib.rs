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
pub use backend::{
    AgentBackend, BackendBindingEvidence, BackendCapabilities, BackendCommandEvidence,
    BackendEvent, BackendFailure, BackendFailureKind, BackendIdentity, BackendOutcomeEvidence,
    BackendPoll, BackendRequestEvidence, BackendResumeTarget, BackendScriptStep, BackendStopHandle,
    CodexBackend, CodexBackendConfig, CodexSkillReferenceProvider, ContinuationStrategy,
    ModelReplay, ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ModelReplayTool, NativeModelBackend, NativeModelBackendConfig, NativeModelBackendServices,
    ReplayExecutor, ScriptedBackend,
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
    ConnectorError, ConnectorFailureKind, FunctionTool, ModelConnectorCancellation,
    ModelConnectorEvent, ModelConnectorInputItem, ModelConnectorInputRole, ModelConnectorLimits,
    ModelConnectorPoll, ModelConnectorRequest, ModelConnectorStream, ModelConnectorTerminal,
    ModelConnectorUsage, OpenAiChatCompletionsConnector, OpenAiResponsesConnector,
    ReasoningChannel, ReasoningEffort, ResponseTerminal, ResponsesCancellation,
    ResponsesConnectorLimits, ResponsesEvent, ResponsesInputItem, ResponsesInputRole,
    ResponsesPoll, ResponsesRequest, ResponsesStream, ResponsesUsage,
};
pub use model_service::{
    AccountId, ApiCredential, ApiDialect, BindingProfileDigest, BindingProfileSchema,
    BindingProfileV1, ConnectionCommit, ConnectionCredentialAction, ConnectionOperationError,
    ConnectionOperationExecutionError, ConnectionOperationExecutionOutcome,
    ConnectionOperationJournalEntry, ConnectionOperationJournalRepository, ConnectionOperationKind,
    ConnectionOperationPhase, ConnectionOperationRecovery, ConnectionOperationRepositoryKind,
    ConnectionRepository, ConnectionRepositoryError, ConnectionRevision, ConnectionSnapshot,
    ConnectorId, CredentialCommit, CredentialMutationAction, CredentialRepository,
    CredentialRevision, CredentialSnapshot, CredentialStore, EffectiveModelBinding,
    LocalConnectionOperationGuard, LocalConnectionOperationJournal,
    LocalConnectionOperationRepositories, LocalConnectionOperationSession,
    LocalConnectionRepository, LocalCredentialRepository, LocalCredentialStore,
    LocalCredentialStoreError, ModelCatalog, ModelCatalogEntry, ModelContextProfile, ModelId,
    ModelSelection, ModelSelectionChoice, ModelSelectionController, ModelServiceError,
    ModelTokenCounter, ModelTokenCounterError, NormalizedEndpoint, PreparedConnectionMutation,
    PreparedCredentialMutation, ProviderId, StartupPolicy, StartupSelectionSources, StartupTarget,
    plan_connection_recovery, resolve_startup_target,
};
pub use request_trace::{RequestTraceEntry, RequestTraceRecord};
pub use runtime::{AgentRuntime, RuntimeError, RuntimePoll};
pub use session::{
    ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionDescriptor, SessionId,
    SessionIdError, SessionIdGenerationError, SessionStartTime, TurnId, TurnRef,
};
pub use skill_reference::{
    SkillAvailability, SkillReference, SkillReferenceCandidate, SkillReferenceProvider,
    SkillReferenceProviderPoll, SkillReferenceScope, SkillReferenceSearchRequest,
    SkillReferenceSearchStatus, SkillReferenceSearchUpdate,
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
