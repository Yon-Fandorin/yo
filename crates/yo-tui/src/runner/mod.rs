//! Narrow live TUI facade for the `yo` application entry point.

mod agent;
mod error;
mod state;
mod unix;

pub use agent::{AgentAction, AgentConnection, DispatchOutcome, PendingDispatch};
pub use error::RunError;
pub use unix::{run, run_with_mode};

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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunOutcome {
    reason: ExitReason,
}

impl RunOutcome {
    /// Returns why the session ended normally.
    #[must_use]
    pub const fn reason(self) -> ExitReason {
        self.reason
    }

    const fn user_requested() -> Self {
        Self {
            reason: ExitReason::UserRequested,
        }
    }

    const fn termination_requested() -> Self {
        Self {
            reason: ExitReason::TerminationRequested,
        }
    }
}

#[cfg(test)]
mod tests;
