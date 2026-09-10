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
    CompactContext {
        guidance: Option<String>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WireActivityResponse {
    Approval {
        decision: WireApprovalDecision,
    },
    UserInput {
        input: WireUserInput,
    },
    QuestionAnswer {
        choice: u32,
        notes: WireUserInput,
    },
    PreviousQuestion {
        choice: Option<u32>,
        draft: WireUserInput,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WireApprovalDecision {
    Approved,
    Declined,
    Offered(u32),
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
            AgentCommand::CompactContext { guidance } => Self::CompactContext {
                guidance: guidance.clone(),
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
            WireCommand::CompactContext { guidance } => {
                (AgentCommand::CompactContext { guidance }, None)
            },
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
        if response.has_resolved_skill() || response_has_images(response) {
            return Err(JournalCodecError::new(
                "Activity responses require structured input v1",
            ));
        }
        Ok(match response {
            ActivityResponse::Approval(decision) => Self::Approval {
                decision: match decision {
                    ApprovalDecision::Approved => WireApprovalDecision::Approved,
                    ApprovalDecision::Declined => WireApprovalDecision::Declined,
                    ApprovalDecision::Offered(choice) => WireApprovalDecision::Offered(*choice),
                },
            },
            ActivityResponse::QuestionAnswer { choice, notes } => Self::QuestionAnswer {
                choice: *choice,
                notes: WireUserInput::try_from(notes)?,
            },
            ActivityResponse::PreviousQuestion { choice, draft } => Self::PreviousQuestion {
                choice: *choice,
                draft: WireUserInput::try_from(draft)?,
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
        let response = match response {
            WireActivityResponse::Approval { decision } => Self::Approval(match decision {
                WireApprovalDecision::Approved => ApprovalDecision::Approved,
                WireApprovalDecision::Declined => ApprovalDecision::Declined,
                WireApprovalDecision::Offered(choice) => ApprovalDecision::Offered(choice),
            }),
            WireActivityResponse::PreviousQuestion { choice, draft } => Self::PreviousQuestion {
                choice,
                draft: draft.try_into()?,
            },
            WireActivityResponse::UserInput { input } => Self::UserInput(input.try_into()?),
            WireActivityResponse::QuestionAnswer { choice, notes } => Self::QuestionAnswer {
                choice,
                notes: notes.try_into()?,
            },
        };
        if response.has_resolved_skill() || response_has_images(&response) {
            return Err(JournalCodecError::new(
                "Activity responses require structured input v1",
            ));
        }
        Ok(response)
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

fn response_has_images(response: &ActivityResponse) -> bool {
    match response {
        ActivityResponse::Approval(_) => false,
        ActivityResponse::UserInput(input)
        | ActivityResponse::QuestionAnswer { notes: input, .. }
        | ActivityResponse::PreviousQuestion { draft: input, .. } => !input.images().is_empty(),
    }
}
