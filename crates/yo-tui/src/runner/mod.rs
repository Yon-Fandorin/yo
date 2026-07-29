//! Narrow live TUI facade for the `yo` application entry point.

mod agent;
mod error;
mod session;
mod state;
mod unix;

pub use agent::{AgentAction, AgentConnection, DispatchOutcome, PendingDispatch};
pub use error::RunError;
pub use session::TuiSession;
pub use unix::{run, run_session_with_mode, run_with_mode};

/// Terminal presentation selected before the live session acquires terminal state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PresentationMode {
    /// Renders on the main screen and preserves native terminal scrollback.
    Inline,
    /// Owns the alternate screen for the duration of the live session.
    Fullscreen,
}

/// A process-host termination observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationEvent {
    /// No process termination has been requested.
    None,
    /// The host requested termination after terminal cleanup.
    Requested,
}

/// Supplies process-host termination observations without exposing OS signals.
pub trait TerminationSource {
    /// Polls the current process termination state.
    fn poll_termination(&mut self) -> TerminationEvent;
}

impl<S> TerminationSource for &mut S
where
    S: TerminationSource + ?Sized,
{
    fn poll_termination(&mut self) -> TerminationEvent {
        (**self).poll_termination()
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

/// The result of one terminal ownership generation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TerminalOutcome {
    /// The application session completed and should be shut down.
    Exited(RunOutcome),
    /// Terminal state was restored and the process host should suspend.
    SuspendRequested,
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
