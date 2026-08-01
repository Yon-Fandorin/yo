use std::fmt;

use crate::{AgentEvent, AgentRejection, BackendEvent, BackendFailure};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    CommandRejected(AgentRejection),
    Backend {
        failure: BackendFailure,
        terminal_events: Vec<AgentEvent>,
    },
    EventRejected {
        event: Box<BackendEvent>,
        rejection: AgentRejection,
        terminal_events: Vec<AgentEvent>,
    },
    StateDiverged(AgentRejection),
}

impl RuntimeError {
    pub(super) fn backend(failure: BackendFailure) -> Self {
        Self::Backend {
            failure,
            terminal_events: Vec::new(),
        }
    }

    pub fn terminal_events(&self) -> &[AgentEvent] {
        match self {
            Self::Backend {
                terminal_events, ..
            }
            | Self::EventRejected {
                terminal_events, ..
            } => terminal_events,
            Self::CommandRejected(_) | Self::StateDiverged(_) => &[],
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandRejected(rejection) => write!(formatter, "command rejected: {rejection}"),
            Self::Backend { failure, .. } => write!(formatter, "backend: {failure}"),
            Self::EventRejected { rejection, .. } => {
                write!(formatter, "backend event rejected: {rejection}")
            },
            Self::StateDiverged(rejection) => {
                write!(formatter, "runtime state diverged: {rejection}")
            },
        }
    }
}

impl std::error::Error for RuntimeError {}
