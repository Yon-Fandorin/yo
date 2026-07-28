//! Frontend-independent agent execution semantics for yo.

mod agent_session;
mod backend;
mod command;
mod engine;
mod event;
mod runtime;
mod session;

pub use agent_session::{
    AgentIntent, AgentSession, AgentSessionError, CommandAdmission, PendingCommand,
};
pub use backend::{
    AgentBackend, BackendCapabilities, BackendEvent, BackendFailure, BackendFailureKind,
    BackendPoll, BackendScriptStep, BackendStopHandle, CodexBackend, CodexBackendConfig,
    ScriptedBackend,
};
pub use command::{ActivityResponse, AgentCommand, ApprovalDecision, UserInput};
pub use engine::{AgentEngine, AgentRejection, ExpectedResponse, ResponseKind};
pub use event::{ActivityKind, ActivityOutcome, ActivityUpdate, AgentEvent, Failure, TurnOutcome};
pub use runtime::{AgentRuntime, RuntimeError, RuntimePoll};
pub use session::{
    ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionId, TurnId, TurnRef,
};

#[cfg(test)]
mod tests;
