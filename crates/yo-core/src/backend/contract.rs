use std::fmt;

use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentCommand, TurnOutcome, TurnRef,
};

/// Frontend-independent port implemented by an agent provider adapter.
///
/// Provider wire types remain behind this boundary. An implementation may use a worker internally,
/// but these methods themselves do not expose transport or process details.
pub trait AgentBackend {
    /// Returns provider-neutral capabilities fixed for this initialized backend.
    fn capabilities(&self) -> BackendCapabilities;

    /// Executes a command far enough to know whether the backend accepted it.
    ///
    /// Streamed work caused by an accepted command is observed through [`Self::poll_event`].
    fn execute_command(&mut self, command: AgentCommand) -> Result<(), BackendFailure>;

    /// Observes one already available semantic event without waiting for future backend work.
    fn poll_event(&mut self) -> Result<BackendPoll, BackendFailure>;

    /// Explicitly releases backend-owned resources.
    ///
    /// Implementations must make repeated calls safe. A successful call makes later polling return
    /// [`BackendPoll::Closed`] and later commands fail explicitly.
    fn shutdown(&mut self) -> Result<(), BackendFailure>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BackendCapabilities {
    steer: bool,
}

impl BackendCapabilities {
    pub const fn none() -> Self {
        Self { steer: false }
    }

    pub const fn with_steer(mut self) -> Self {
        self.steer = true;
        self
    }

    pub const fn supports_steer(self) -> bool {
        self.steer
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendPoll {
    Pending,
    Event(BackendEvent),
    Closed,
}

/// Semantic output a backend may submit to `yo-core`.
///
/// Session and Turn creation remain core command transitions, so a backend cannot fabricate their
/// corresponding frontend events through this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendEvent {
    ActivityStarted {
        activity: ActivityRef,
        kind: ActivityKind,
    },
    ActivityUpdated {
        activity: ActivityRef,
        update: ActivityUpdate,
    },
    ActivityFinished {
        activity: ActivityRef,
        outcome: ActivityOutcome,
    },
    TurnFinished {
        turn: TurnRef,
        outcome: TurnOutcome,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendFailureKind {
    Unavailable,
    Initialization,
    Session,
    Unsupported,
    Protocol,
    ProcessExit,
    Turn,
    Cleanup,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendFailure {
    kind: BackendFailureKind,
    message: String,
}

impl BackendFailure {
    pub fn new(kind: BackendFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> BackendFailureKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BackendFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for BackendFailure {}
