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
    /// Appends one ordered fragment to the Activity's current text.
    TextDelta(String),
    /// Replaces the Activity's current text with an authoritative snapshot.
    TextSnapshot(String),
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
    code: Option<String>,
    message: String,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Result<Self, &'static str> {
        let code = code.into();
        if code.is_empty()
            || code.len() > 128
            || !code.is_ascii()
            || code
                .chars()
                .any(|character| character.is_ascii_whitespace() || character.is_ascii_control())
        {
            return Err("failure code must be a non-empty bounded ASCII identifier");
        }
        self.code = Some(code);
        Ok(self)
    }

    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}
