//! Frontend-independent agent execution semantics for yo.

mod command;
mod event;
mod session;

pub use command::{ActivityResponse, AgentCommand, ApprovalDecision, UserInput};
pub use event::{ActivityKind, ActivityOutcome, ActivityUpdate, AgentEvent, Failure, TurnOutcome};
pub use session::{
    ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionId, TurnId, TurnRef,
};

#[cfg(test)]
mod tests;
