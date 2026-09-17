use super::outcome::{ActivityOutcome, TurnOutcome};
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
    /// 현재 Activity 텍스트에 순서가 보장된 한 조각을 덧붙입니다.
    TextDelta(String),
    /// 현재 Activity 텍스트를 권위 있는 snapshot으로 교체합니다.
    TextSnapshot(String),
}
