//! Deterministic completed cell state for yo terminal interfaces.

pub mod html;
pub(crate) mod input;
pub(crate) mod layout;
pub(crate) mod prompt;
#[cfg(unix)]
mod runner;
pub(crate) mod shell;
pub mod surface;
pub mod terminal;
pub(crate) mod text;
pub(crate) mod transcript;

#[cfg(unix)]
pub use runner::{
    AgentAction, AgentConnection, DispatchOutcome, ExitReason, PendingDispatch, PresentationMode,
    RunError, RunOutcome, TerminalOutcome, TerminationEvent, TerminationSource, TuiSession, run,
    run_session_with_mode, run_with_mode,
};
