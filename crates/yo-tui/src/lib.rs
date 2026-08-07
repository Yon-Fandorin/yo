//! Deterministic completed cell state for yo terminal interfaces.

pub(crate) mod appearance;
pub mod html;
pub(crate) mod input;
pub(crate) mod layout;
pub(crate) mod overlay;
pub mod plain;
pub(crate) mod prompt;
#[cfg(unix)]
mod runner;
pub(crate) mod shell;
pub mod surface;
pub mod terminal;
pub(crate) mod text;
pub(crate) mod transcript;

pub use appearance::{ColorCapability, GlyphProfile, MotionPreference};
#[cfg(unix)]
pub use runner::{
    AgentAction, AgentConnection, AgentPoll, ArchivedProjectionError, ArchivedSessionView,
    DispatchOutcome, ExitReason, FrameRateLimit, PendingDispatch, PresentationMode, RunError,
    RunOutcome, TerminalOutcome, TerminationEvent, TerminationSource, TuiSession, TuiSessionInfo,
    WorkspaceReferenceConnection, WorkspaceReferencePoll, project_archived_session, run,
    run_session_with_mode, run_with_mode,
};
