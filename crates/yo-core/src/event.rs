use crate::{ActivityRef, RequestId, SessionId, TurnRef};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentEvent {
    SessionCreated {
        session_id: SessionId,
    },
    TurnStarted {
        turn: TurnRef,
    },
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
pub enum ActivityKind {
    ModelWork,
    AgentMessage,
    ToolCall,
    ToolResult,
    FileChange,
    ApprovalRequest { request_id: RequestId },
    ApprovalResponse { request_id: RequestId },
    UserInputRequest { request_id: RequestId },
    UserInputResponse { request_id: RequestId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityUpdate {
    TextDelta(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityOutcome {
    Completed,
    Interrupted,
    Failed(Failure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TurnOutcome {
    Completed,
    Interrupted,
    Failed(Failure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    message: String,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}
