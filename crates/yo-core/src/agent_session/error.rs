use std::{error::Error, fmt};

use crate::{BackendFailure, RuntimeError};

/// Failure while starting, using, or stopping an [`super::AgentSession`].
#[derive(Clone, Debug)]
pub enum AgentSessionError {
    Runtime(RuntimeError),
    BackendCleanup(BackendFailure),
    StartAndCleanup {
        start: Box<RuntimeError>,
        cleanup: Box<RuntimeError>,
    },
    Multiple {
        primary: Box<AgentSessionError>,
        additional: Box<AgentSessionError>,
    },
    NoActiveTurn,
    NoOutstandingRequest,
    TurnNoLongerActive,
    TurnInterruptPending,
    TurnIdExhausted,
    WorkerUnavailable(String),
    WorkerShutdownTimedOut,
    WorkerPanicked,
}

impl fmt::Display for AgentSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => error.fmt(formatter),
            Self::BackendCleanup(error) => write!(formatter, "backend cleanup: {error}"),
            Self::StartAndCleanup { start, cleanup } => {
                write!(
                    formatter,
                    "{start}; additionally, startup cleanup failed: {cleanup}"
                )
            },
            Self::Multiple {
                primary,
                additional,
            } => {
                write!(formatter, "{primary}; additionally, {additional}")
            },
            Self::NoActiveTurn => formatter.write_str("no Turn is active"),
            Self::NoOutstandingRequest => {
                formatter.write_str("the Activity request is no longer outstanding")
            },
            Self::TurnNoLongerActive => {
                formatter.write_str("the command's target Turn is no longer active")
            },
            Self::TurnInterruptPending => {
                formatter.write_str("the active Turn is already being interrupted")
            },
            Self::TurnIdExhausted => formatter.write_str("the Turn identity space was exhausted"),
            Self::WorkerUnavailable(detail) => {
                write!(
                    formatter,
                    "the agent runtime worker is unavailable: {detail}"
                )
            },
            Self::WorkerShutdownTimedOut => {
                formatter.write_str("the agent runtime worker did not stop within 3 seconds")
            },
            Self::WorkerPanicked => formatter.write_str("the agent runtime worker panicked"),
        }
    }
}

impl Error for AgentSessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Runtime(error) => Some(error),
            Self::BackendCleanup(error) => Some(error),
            Self::StartAndCleanup { start, .. } => Some(start.as_ref()),
            Self::Multiple { primary, .. } => Some(primary),
            Self::NoActiveTurn
            | Self::NoOutstandingRequest
            | Self::TurnNoLongerActive
            | Self::TurnInterruptPending
            | Self::TurnIdExhausted
            | Self::WorkerUnavailable(_)
            | Self::WorkerShutdownTimedOut
            | Self::WorkerPanicked => None,
        }
    }
}
