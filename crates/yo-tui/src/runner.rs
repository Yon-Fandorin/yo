//! Narrow live TUI facade for the `yo` application entry point.

mod agent;
mod archival;
mod chat;
mod error;
mod frame;
mod model;
mod preview_agent;
mod publication;
mod session;
mod skill;
mod source_schedule;
mod state;
mod unix;
mod view;
mod workspace;

pub use agent::{AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch};
pub use archival::{
    ArchivedContentPolicy, ArchivedProjectionError, ArchivedProjectionOptions, ArchivedSessionView,
    project_archived_session, project_archived_session_with_options, project_archived_usage,
};
pub use error::RunError;
pub use frame::FrameRateLimit;
pub use session::{
    PublicationRecoveryEvidence, PublicationRecoveryKind, ResumeSessionEntry, TuiDocument,
    TuiSession, TuiSessionInfo, TuiStatusError, TuiStatusLine,
};
pub use skill::{SkillReferenceConnection, SkillReferencePoll};
pub use unix::{run, run_session_with_mode, run_with_mode};
pub use workspace::{WorkspaceReferenceConnection, WorkspaceReferencePoll};

use crate::overlay::OverlayInstanceToken;

/// Terminal presentation selected before the live session acquires terminal state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum PresentationMode {
    /// Renders on the main screen and preserves native terminal scrollback.
    #[default]
    Inline,
    /// Owns the alternate screen for the duration of the live session.
    Fullscreen,
}

/// A process-host termination observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationEvent {
    /// The host requested termination after terminal cleanup.
    Requested,
}

/// Supplies process-host termination readiness without exposing OS signals.
pub trait TerminationSource {
    /// Registers the frontend task and observes a pending termination request.
    fn poll_termination(
        &mut self,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<TerminationEvent>;
}

impl<S> TerminationSource for &mut S
where
    S: TerminationSource + ?Sized,
{
    fn poll_termination(
        &mut self,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<TerminationEvent> {
        (**self).poll_termination(context)
    }
}

/// Why a live TUI session returned normally.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExitReason {
    /// The user completed the configured terminal exit gesture.
    UserRequested,
    /// The process host requested termination.
    TerminationRequested,
}

/// The normal result of a completed live TUI session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutcome {
    reason: ExitReason,
    output: Option<String>,
}

/// Identifies one displayed historical-fork catalog without exposing its overlay internals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForkPickerToken(OverlayInstanceToken);

/// The result of one terminal ownership generation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TerminalOutcome {
    /// The application session completed and should be shut down.
    Exited(RunOutcome),
    /// Terminal state was restored and the process host should suspend.
    SuspendRequested,
    /// The frontend selected a fully qualified model binding for this Session.
    ModelSelectionRequested(yo_core::ModelPickerTarget),
    /// The idle frontend requested a new independent session after terminal restoration.
    NewSessionRequested,
    /// Requests an exact branch of the current durable conversation after terminal restoration.
    ForkSessionRequested,
    /// Requests a frozen catalog of historical boundaries for the current idle Session.
    ForkPickerRequested,
    /// Requests one row from the host's catalog bound to this exact picker token.
    ForkBoundaryRequested {
        /// Exact picker generation bound to the host's frozen catalog.
        picker: ForkPickerToken,
        /// Zero-based row in that catalog, never a journal sequence or authority claim.
        index: usize,
    },
    /// Requests a bounded read-only Session tree after terminal restoration.
    SessionTreeRequested,
    /// Requests a saved session, or a host-owned picker when no identity was supplied.
    ResumeSessionRequested(Option<yo_core::SessionId>),
}

impl RunOutcome {
    /// Returns why the session ended normally.
    #[must_use]
    pub const fn reason(&self) -> ExitReason {
        self.reason
    }

    /// Returns terminal-independent session output prepared for the caller.
    #[must_use]
    pub fn output(&self) -> Option<&str> {
        self.output.as_deref()
    }

    const fn user_requested(output: Option<String>) -> Self {
        Self {
            reason: ExitReason::UserRequested,
            output,
        }
    }

    const fn termination_requested(output: Option<String>) -> Self {
        Self {
            reason: ExitReason::TerminationRequested,
            output,
        }
    }
}

#[cfg(test)]
mod tests;
