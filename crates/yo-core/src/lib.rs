//! Frontend-independent agent execution semantics for yo.

mod agent_session;
mod backend;
mod command;
mod engine;
mod event;
mod host;
mod journal;
mod runtime;
mod session;
pub mod session_repository;

pub use agent_session::{
    AgentIntent, AgentSession, AgentSessionError, AgentSessionPoll, CommandAdmission,
    PendingCommand,
};
pub use backend::{
    AgentBackend, BackendCapabilities, BackendEvent, BackendFailure, BackendFailureKind,
    BackendPoll, BackendScriptStep, BackendStopHandle, CodexBackend, CodexBackendConfig,
    ScriptedBackend,
};
pub use command::{ActivityResponse, AgentCommand, ApprovalDecision, UserInput};
pub use engine::{AgentEngine, AgentRejection, ExpectedResponse, ResponseKind};
pub use event::{ActivityKind, ActivityOutcome, ActivityUpdate, AgentEvent, Failure, TurnOutcome};
pub use host::{
    LocalWorkspaceHostIdentity, LocalWorkspaceHostIdentityError, WorkspaceHostId,
    WorkspaceHostIdError, WorkspaceHostIdGenerationError,
};
pub use journal::{
    DurabilityGapCause, JournalDurability, JournalSequence, TranscriptEntry, TranscriptObservation,
    TranscriptObservationEntry, TranscriptObservationSequence, TranscriptObservationSlice,
    TranscriptReader, TranscriptRecord, TranscriptSlice,
};
pub use runtime::{AgentRuntime, RuntimeError, RuntimePoll};
pub use session::{
    ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionId, SessionIdError,
    SessionIdGenerationError, TurnId, TurnRef,
};

#[cfg(test)]
pub(crate) fn fixture_session(value: u64) -> SessionId {
    let uuid = uuid::Uuid::from_u128(0x0189_0f00_0000_7000_8000_0000_0000_0000 | u128::from(value));
    SessionId::from_uuid(uuid).expect("the test Session fixture is a UUIDv7")
}

#[cfg(test)]
mod tests;
