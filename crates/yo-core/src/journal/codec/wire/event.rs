use serde::{Deserialize, Serialize};

use super::{
    JournalCodecError,
    identity::{WireActivityRef, WireSessionId, WireTurnRef, request_id_from, session_id_from},
};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRef, ActivityUpdate, AgentEvent, Failure, TurnOutcome,
    TurnRef,
};

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WireEvent {
    SessionCreated {
        session_id: WireSessionId,
    },
    TurnStarted {
        turn: WireTurnRef,
    },
    ActivityStarted {
        activity: WireActivityRef,
        kind: WireActivityKind,
    },
    ActivityUpdated {
        activity: WireActivityRef,
        update: WireActivityUpdate,
    },
    ActivityFinished {
        activity: WireActivityRef,
        outcome: WireOutcome,
    },
    TurnFinished {
        turn: WireTurnRef,
        outcome: WireOutcome,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WireActivityKind {
    ModelWork,
    AgentMessage,
    ToolCall,
    ToolResult,
    FileChange,
    ApprovalRequest { request_id: u64 },
    ApprovalResponse { request_id: u64 },
    UserInputRequest { request_id: u64 },
    UserInputResponse { request_id: u64 },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WireActivityUpdate {
    TextDelta { text: String },
    TextSnapshot { text: String },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WireOutcome {
    Completed,
    Interrupted,
    Failed { message: String },
}

impl From<&AgentEvent> for WireEvent {
    fn from(event: &AgentEvent) -> Self {
        match event {
            AgentEvent::SessionCreated { session_id } => Self::SessionCreated {
                session_id: WireSessionId::from(*session_id),
            },
            AgentEvent::TurnStarted { turn } => Self::TurnStarted {
                turn: WireTurnRef::from(*turn),
            },
            AgentEvent::ActivityStarted { activity, kind } => Self::ActivityStarted {
                activity: WireActivityRef::from(*activity),
                kind: WireActivityKind::from(*kind),
            },
            AgentEvent::ActivityUpdated { activity, update } => Self::ActivityUpdated {
                activity: WireActivityRef::from(*activity),
                update: WireActivityUpdate::from(update),
            },
            AgentEvent::ActivityFinished { activity, outcome } => Self::ActivityFinished {
                activity: WireActivityRef::from(*activity),
                outcome: WireOutcome::from(outcome),
            },
            AgentEvent::TurnFinished { turn, outcome } => Self::TurnFinished {
                turn: WireTurnRef::from(*turn),
                outcome: WireOutcome::from(outcome),
            },
        }
    }
}

impl TryFrom<WireEvent> for AgentEvent {
    type Error = JournalCodecError;

    fn try_from(event: WireEvent) -> Result<Self, Self::Error> {
        match event {
            WireEvent::SessionCreated { session_id } => Ok(Self::SessionCreated {
                session_id: session_id_from(session_id, "Session")?,
            }),
            WireEvent::TurnStarted { turn } => Ok(Self::TurnStarted {
                turn: TurnRef::try_from(turn)?,
            }),
            WireEvent::ActivityStarted { activity, kind } => Ok(Self::ActivityStarted {
                activity: ActivityRef::try_from(activity)?,
                kind: ActivityKind::try_from(kind)?,
            }),
            WireEvent::ActivityUpdated { activity, update } => Ok(Self::ActivityUpdated {
                activity: ActivityRef::try_from(activity)?,
                update: ActivityUpdate::from(update),
            }),
            WireEvent::ActivityFinished { activity, outcome } => Ok(Self::ActivityFinished {
                activity: ActivityRef::try_from(activity)?,
                outcome: ActivityOutcome::from(outcome),
            }),
            WireEvent::TurnFinished { turn, outcome } => Ok(Self::TurnFinished {
                turn: TurnRef::try_from(turn)?,
                outcome: TurnOutcome::from(outcome),
            }),
        }
    }
}

impl From<ActivityKind> for WireActivityKind {
    fn from(kind: ActivityKind) -> Self {
        match kind {
            ActivityKind::ModelWork => Self::ModelWork,
            ActivityKind::AgentMessage => Self::AgentMessage,
            ActivityKind::ToolCall => Self::ToolCall,
            ActivityKind::ToolResult => Self::ToolResult,
            ActivityKind::FileChange => Self::FileChange,
            ActivityKind::ApprovalRequest { request_id } => Self::ApprovalRequest {
                request_id: request_id.get().get(),
            },
            ActivityKind::ApprovalResponse { request_id } => Self::ApprovalResponse {
                request_id: request_id.get().get(),
            },
            ActivityKind::UserInputRequest { request_id } => Self::UserInputRequest {
                request_id: request_id.get().get(),
            },
            ActivityKind::UserInputResponse { request_id } => Self::UserInputResponse {
                request_id: request_id.get().get(),
            },
        }
    }
}

impl TryFrom<WireActivityKind> for ActivityKind {
    type Error = JournalCodecError;

    fn try_from(kind: WireActivityKind) -> Result<Self, Self::Error> {
        match kind {
            WireActivityKind::ModelWork => Ok(Self::ModelWork),
            WireActivityKind::AgentMessage => Ok(Self::AgentMessage),
            WireActivityKind::ToolCall => Ok(Self::ToolCall),
            WireActivityKind::ToolResult => Ok(Self::ToolResult),
            WireActivityKind::FileChange => Ok(Self::FileChange),
            WireActivityKind::ApprovalRequest { request_id } => Ok(Self::ApprovalRequest {
                request_id: request_id_from(request_id)?,
            }),
            WireActivityKind::ApprovalResponse { request_id } => Ok(Self::ApprovalResponse {
                request_id: request_id_from(request_id)?,
            }),
            WireActivityKind::UserInputRequest { request_id } => Ok(Self::UserInputRequest {
                request_id: request_id_from(request_id)?,
            }),
            WireActivityKind::UserInputResponse { request_id } => Ok(Self::UserInputResponse {
                request_id: request_id_from(request_id)?,
            }),
        }
    }
}

impl From<&ActivityUpdate> for WireActivityUpdate {
    fn from(update: &ActivityUpdate) -> Self {
        match update {
            ActivityUpdate::TextDelta(text) => Self::TextDelta { text: text.clone() },
            ActivityUpdate::TextSnapshot(text) => Self::TextSnapshot { text: text.clone() },
        }
    }
}

impl From<WireActivityUpdate> for ActivityUpdate {
    fn from(update: WireActivityUpdate) -> Self {
        match update {
            WireActivityUpdate::TextDelta { text } => Self::TextDelta(text),
            WireActivityUpdate::TextSnapshot { text } => Self::TextSnapshot(text),
        }
    }
}

impl From<&ActivityOutcome> for WireOutcome {
    fn from(outcome: &ActivityOutcome) -> Self {
        match outcome {
            ActivityOutcome::Completed => Self::Completed,
            ActivityOutcome::Interrupted => Self::Interrupted,
            ActivityOutcome::Failed(failure) => Self::Failed {
                message: failure.message().to_owned(),
            },
        }
    }
}

impl From<&TurnOutcome> for WireOutcome {
    fn from(outcome: &TurnOutcome) -> Self {
        match outcome {
            TurnOutcome::Completed => Self::Completed,
            TurnOutcome::Interrupted => Self::Interrupted,
            TurnOutcome::Failed(failure) => Self::Failed {
                message: failure.message().to_owned(),
            },
        }
    }
}

impl From<WireOutcome> for ActivityOutcome {
    fn from(outcome: WireOutcome) -> Self {
        match outcome {
            WireOutcome::Completed => Self::Completed,
            WireOutcome::Interrupted => Self::Interrupted,
            WireOutcome::Failed { message } => Self::Failed(Failure::new(message)),
        }
    }
}

impl From<WireOutcome> for TurnOutcome {
    fn from(outcome: WireOutcome) -> Self {
        match outcome {
            WireOutcome::Completed => Self::Completed,
            WireOutcome::Interrupted => Self::Interrupted,
            WireOutcome::Failed { message } => Self::Failed(Failure::new(message)),
        }
    }
}
