use serde::{Deserialize, Serialize};

use super::{
    JournalCodecError,
    identity::{WireActivityRequestRef, WireSessionId, WireTurnRef, session_id_from},
    input::WireUserInput,
};
use crate::{
    ActivityRequestRef, ActivityResponse, AgentCommand, ApprovalDecision, SubmissionId, TurnRef,
    journal::CommittedCommand,
};

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WireCommand {
    CreateSession {
        session_id: WireSessionId,
    },
    StartTurn {
        turn: WireTurnRef,
        submission_id: String,
        input: WireUserInput,
    },
    SteerTurn {
        turn: WireTurnRef,
        submission_id: String,
        input: WireUserInput,
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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WireActivityResponse {
    Approval { decision: WireApprovalDecision },
    UserInput { input: WireUserInput },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireApprovalDecision {
    Approved,
    Declined,
}

impl TryFrom<&CommittedCommand> for WireCommand {
    type Error = JournalCodecError;

    fn try_from(committed: &CommittedCommand) -> Result<Self, Self::Error> {
        let command = committed.command();
        let wire = match command {
            AgentCommand::CreateSession { session_id } => Self::CreateSession {
                session_id: WireSessionId::from(*session_id),
            },
            AgentCommand::StartTurn { turn, input } => Self::StartTurn {
                turn: WireTurnRef::from(*turn),
                submission_id: required_submission_id(committed)?.to_string(),
                input: WireUserInput::try_from(input)?,
            },
            AgentCommand::SteerTurn { turn, input } => Self::SteerTurn {
                turn: WireTurnRef::from(*turn),
                submission_id: required_submission_id(committed)?.to_string(),
                input: WireUserInput::try_from(input)?,
            },
            AgentCommand::RespondToActivity { request, response } => Self::RespondToActivity {
                request: WireActivityRequestRef::from(*request),
                response: WireActivityResponse::try_from(response)?,
            },
            AgentCommand::InterruptTurn { turn } => Self::InterruptTurn {
                turn: WireTurnRef::from(*turn),
            },
        };
        Ok(wire)
    }
}

impl TryFrom<WireCommand> for CommittedCommand {
    type Error = JournalCodecError;

    fn try_from(command: WireCommand) -> Result<Self, Self::Error> {
        let (command, submission_id) = match command {
            WireCommand::CreateSession { session_id } => (
                AgentCommand::CreateSession {
                    session_id: session_id_from(session_id, "Session")?,
                },
                None,
            ),
            WireCommand::StartTurn {
                turn,
                submission_id,
                input,
            } => (
                AgentCommand::StartTurn {
                    turn: TurnRef::try_from(turn)?,
                    input: input.try_into()?,
                },
                Some(parse_submission_id(&submission_id)?),
            ),
            WireCommand::SteerTurn {
                turn,
                submission_id,
                input,
            } => (
                AgentCommand::SteerTurn {
                    turn: TurnRef::try_from(turn)?,
                    input: input.try_into()?,
                },
                Some(parse_submission_id(&submission_id)?),
            ),
            WireCommand::RespondToActivity { request, response } => (
                AgentCommand::RespondToActivity {
                    request: ActivityRequestRef::try_from(request)?,
                    response: ActivityResponse::try_from(response)?,
                },
                None,
            ),
            WireCommand::InterruptTurn { turn } => (
                AgentCommand::InterruptTurn {
                    turn: TurnRef::try_from(turn)?,
                },
                None,
            ),
        };
        match submission_id {
            Some(submission_id) => Self::submission(command, submission_id),
            None => Self::uncorrelated(command),
        }
        .ok_or_else(|| JournalCodecError::new("command correlation shape is invalid"))
    }
}

impl TryFrom<&ActivityResponse> for WireActivityResponse {
    type Error = JournalCodecError;

    fn try_from(response: &ActivityResponse) -> Result<Self, Self::Error> {
        Ok(match response {
            ActivityResponse::Approval(decision) => Self::Approval {
                decision: match decision {
                    ApprovalDecision::Approved => WireApprovalDecision::Approved,
                    ApprovalDecision::Declined => WireApprovalDecision::Declined,
                },
            },
            ActivityResponse::UserInput(input) => Self::UserInput {
                input: WireUserInput::try_from(input)?,
            },
        })
    }
}

impl TryFrom<WireActivityResponse> for ActivityResponse {
    type Error = JournalCodecError;

    fn try_from(response: WireActivityResponse) -> Result<Self, Self::Error> {
        Ok(match response {
            WireActivityResponse::Approval { decision } => Self::Approval(match decision {
                WireApprovalDecision::Approved => ApprovalDecision::Approved,
                WireApprovalDecision::Declined => ApprovalDecision::Declined,
            }),
            WireActivityResponse::UserInput { input } => Self::UserInput(input.try_into()?),
        })
    }
}

fn required_submission_id(command: &CommittedCommand) -> Result<SubmissionId, JournalCodecError> {
    command
        .submission_id()
        .ok_or_else(|| JournalCodecError::new("StartTurn and SteerTurn require a SubmissionId"))
}

fn parse_submission_id(value: &str) -> Result<SubmissionId, JournalCodecError> {
    let parsed: SubmissionId = value
        .parse()
        .map_err(|error| JournalCodecError::new(format!("invalid SubmissionId: {error}")))?;
    if value != parsed.to_string() {
        return Err(JournalCodecError::new(
            "SubmissionId must use canonical lowercase hyphenated UUIDv4 text",
        ));
    }
    Ok(parsed)
}
