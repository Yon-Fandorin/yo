use crate::{
    ActivityRequestRef, AgentCommand, ApprovalDecision, InputSubmission, SubmissionId,
    SubmissionIdGenerationError, SubmissionRejection, TurnRef, UserInput,
};

/// agent Session으로 향하는 frontend intent입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentIntent {
    /// Turn을 시작하거나 활성 Turn을 steer합니다.
    Submit(InputSubmission),
    /// frontend가 관찰한 정확한 Turn을 steer합니다.
    Steer {
        /// submission이 만들어질 때 prompt를 소유한 Turn입니다.
        turn: TurnRef,
        /// 변경할 수 없는 연결 submission입니다.
        submission: InputSubmission,
    },
    /// 활성 Turn의 interrupt를 요청합니다.
    Interrupt,
    /// 선택적인 사용자 guidance와 함께 idle Session을 compaction합니다.
    CompactContext { guidance: Option<String> },
    /// 연결된 approval request 하나에 답합니다.
    RespondToApproval {
        /// 답변할 outstanding request입니다.
        request: ActivityRequestRef,
        /// 사용자의 approval 결정입니다.
        decision: ApprovalDecision,
    },
    /// 연결된 agent-requested input 하나에 답합니다.
    RespondToUserInput {
        /// 답변할 outstanding request입니다.
        request: ActivityRequestRef,
        /// 사용자의 response text입니다.
        input: String,
    },
    /// 현재 draft를 제출하지 않고 이전 question으로 돌아갑니다.
    PreviousQuestion {
        /// 뒤로 가기를 지원하는 정확한 outstanding user-input request입니다.
        request: ActivityRequestRef,
        /// draft에 연결된 선택지의 one-based 번호입니다.
        choice: Option<u32>,
        /// 뒤로 가는 동안 보존할 미제출 answer 또는 notes입니다.
        draft: String,
    },
    /// 연결된 question에 choice와 선택적인 notes로 답합니다.
    RespondToQuestion {
        /// 답변할 outstanding request입니다.
        request: ActivityRequestRef,
        /// question choices 안의 one-based ordinal입니다.
        choice: u32,
        /// 선택한 option과 분리해 보존하는 추가 text입니다.
        notes: String,
    },
}

impl AgentIntent {
    /// 새 correlation identity로 plain-text immutable submission 하나를 만듭니다.
    pub fn submit(text: impl Into<String>) -> Result<Self, SubmissionIdGenerationError> {
        Ok(Self::Submit(InputSubmission::new(
            SubmissionId::new()?,
            UserInput::new(text),
        )))
    }
}

/// agent Session에 intent를 배치한 즉시 결과입니다.
#[derive(Debug, Eq, PartialEq)]
pub enum CommandAdmission {
    /// Session이 전달할 command를 보존했습니다.
    Queued,
    /// Session이 바쁘므로 frontend가 이 작업을 보존하고 다시 시도합니다.
    Backpressured(PendingCommand),
    /// 연결된 submission이 worker에 도달하기 전에 거절되었습니다.
    Rejected {
        /// 이 결과가 소유하는 immutable submission identity입니다.
        id: SubmissionId,
        /// admission이 일어나지 않은 frontend 중립 이유입니다.
        rejection: SubmissionRejection,
    },
}

/// durable 의미를 바꾸지 않은 admitted Session control 결과입니다.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentControlOutcome {
    /// 현재 Session 경계에서 manual context compaction을 사용할 수 없습니다.
    ContextCompactionRejected { detail: String },
}

impl AgentControlOutcome {
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::ContextCompactionRejected { detail } => detail,
        }
    }
}

/// 비차단 backpressure 동안 보존하는 opaque single-use 작업입니다.
#[derive(Debug, Eq, PartialEq)]
pub struct PendingCommand {
    command: AgentCommand,
    submission_id: Option<SubmissionId>,
}

impl PendingCommand {
    pub(super) fn from_command(command: AgentCommand) -> Self {
        Self {
            command,
            submission_id: None,
        }
    }

    pub(super) fn from_submission(command: AgentCommand, submission_id: SubmissionId) -> Self {
        Self {
            command,
            submission_id: Some(submission_id),
        }
    }

    pub(super) fn into_parts(self) -> (AgentCommand, Option<SubmissionId>) {
        (self.command, self.submission_id)
    }

    pub(super) const fn submission_id(&self) -> Option<SubmissionId> {
        self.submission_id
    }

    pub(super) const fn command(&self) -> &AgentCommand {
        &self.command
    }
}
