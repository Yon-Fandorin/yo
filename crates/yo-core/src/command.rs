use crate::{ActivityRequestRef, SessionId, TurnRef};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentCommand {
    CreateSession {
        session_id: SessionId,
    },
    StartTurn {
        turn: TurnRef,
        input: UserInput,
    },
    SteerTurn {
        turn: TurnRef,
        input: UserInput,
    },
    RespondToActivity {
        request: ActivityRequestRef,
        response: ActivityResponse,
    },
    InterruptTurn {
        turn: TurnRef,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActivityResponse {
    Approval(ApprovalDecision),
    UserInput(UserInput),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalDecision {
    Approved,
    Declined,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserInput(String);

impl UserInput {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for UserInput {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl From<&str> for UserInput {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}
