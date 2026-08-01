use serde::{Deserialize, Serialize};

use super::{
    JournalCodecError,
    identity::{WireActivityRequestRef, WireSessionId, WireTurnRef, session_id_from},
};
use crate::{
    ActivityRequestRef, ActivityResponse, AgentCommand, ApprovalDecision, TurnRef, UserInput,
};

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WireCommand {
    CreateSession {
        session_id: WireSessionId,
    },
    StartTurn {
        turn: WireTurnRef,
        input: String,
    },
    SteerTurn {
        turn: WireTurnRef,
        input: String,
    },
    RespondToActivity {
        request: WireActivityRequestRef,
        response: WireActivityResponse,
    },
    InterruptTurn {
        turn: WireTurnRef,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum WireActivityResponse {
    Approval { decision: WireApprovalDecision },
    UserInput { text: String },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireApprovalDecision {
    Approved,
    Declined,
}

impl From<&AgentCommand> for WireCommand {
    fn from(command: &AgentCommand) -> Self {
        match command {
            AgentCommand::CreateSession { session_id } => Self::CreateSession {
                session_id: WireSessionId::from(*session_id),
            },
            AgentCommand::StartTurn { turn, input } => Self::StartTurn {
                turn: WireTurnRef::from(*turn),
                input: input.as_str().to_owned(),
            },
            AgentCommand::SteerTurn { turn, input } => Self::SteerTurn {
                turn: WireTurnRef::from(*turn),
                input: input.as_str().to_owned(),
            },
            AgentCommand::RespondToActivity { request, response } => Self::RespondToActivity {
                request: WireActivityRequestRef::from(*request),
                response: WireActivityResponse::from(response),
            },
            AgentCommand::InterruptTurn { turn } => Self::InterruptTurn {
                turn: WireTurnRef::from(*turn),
            },
        }
    }
}

impl TryFrom<WireCommand> for AgentCommand {
    type Error = JournalCodecError;

    fn try_from(command: WireCommand) -> Result<Self, Self::Error> {
        match command {
            WireCommand::CreateSession { session_id } => Ok(Self::CreateSession {
                session_id: session_id_from(session_id, "Session")?,
            }),
            WireCommand::StartTurn { turn, input } => Ok(Self::StartTurn {
                turn: TurnRef::try_from(turn)?,
                input: UserInput::new(input),
            }),
            WireCommand::SteerTurn { turn, input } => Ok(Self::SteerTurn {
                turn: TurnRef::try_from(turn)?,
                input: UserInput::new(input),
            }),
            WireCommand::RespondToActivity { request, response } => Ok(Self::RespondToActivity {
                request: ActivityRequestRef::try_from(request)?,
                response: ActivityResponse::from(response),
            }),
            WireCommand::InterruptTurn { turn } => Ok(Self::InterruptTurn {
                turn: TurnRef::try_from(turn)?,
            }),
        }
    }
}

impl From<&ActivityResponse> for WireActivityResponse {
    fn from(response: &ActivityResponse) -> Self {
        match response {
            ActivityResponse::Approval(decision) => Self::Approval {
                decision: match decision {
                    ApprovalDecision::Approved => WireApprovalDecision::Approved,
                    ApprovalDecision::Declined => WireApprovalDecision::Declined,
                },
            },
            ActivityResponse::UserInput(input) => Self::UserInput {
                text: input.as_str().to_owned(),
            },
        }
    }
}

impl From<WireActivityResponse> for ActivityResponse {
    fn from(response: WireActivityResponse) -> Self {
        match response {
            WireActivityResponse::Approval { decision } => Self::Approval(match decision {
                WireApprovalDecision::Approved => ApprovalDecision::Approved,
                WireApprovalDecision::Declined => ApprovalDecision::Declined,
            }),
            WireActivityResponse::UserInput { text } => Self::UserInput(UserInput::new(text)),
        }
    }
}
