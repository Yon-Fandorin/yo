use std::fmt;

use crate::{ActivityRef, ActivityRequestRef, RequestId, SessionId, TurnRef};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpectedResponse {
    Approval,
    UserInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseKind {
    Approval,
    UserInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentRejection {
    SessionAlreadyExists {
        existing: SessionId,
        requested: SessionId,
    },
    SessionNotCreated,
    SessionMismatch {
        expected: SessionId,
        actual: SessionId,
    },
    TurnAlreadyActive {
        active: TurnRef,
    },
    DuplicateTurn {
        turn: TurnRef,
    },
    TurnNotActive {
        turn: TurnRef,
    },
    InterruptAlreadyRequested {
        turn: TurnRef,
    },
    DuplicateActivity {
        activity: ActivityRef,
    },
    ActivityNotActive {
        activity: ActivityRef,
    },
    ActivityStillActive {
        activity: ActivityRef,
    },
    DuplicateRequest {
        request: ActivityRequestRef,
    },
    RequestNotFound {
        request: ActivityRequestRef,
    },
    RequestAlreadyAnswered {
        request: ActivityRequestRef,
    },
    RequestStillUnanswered {
        request: ActivityRequestRef,
    },
    ResponseRequestNotFound {
        turn: TurnRef,
        request_id: RequestId,
    },
    ResponseNotAnswered {
        request: ActivityRequestRef,
    },
    ResponseAlreadyRecorded {
        request: ActivityRequestRef,
    },
    ResponseNotRecorded {
        request: ActivityRequestRef,
    },
    ResponseKindMismatch {
        request: ActivityRequestRef,
        expected: ExpectedResponse,
        actual: ResponseKind,
    },
    UnsupportedSteer,
}

impl fmt::Display for AgentRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for AgentRejection {}
