use crate::{ActivityRequestRef, SessionId, TurnRef, UserInput};

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
    CompactContext {
        guidance: Option<String>,
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
