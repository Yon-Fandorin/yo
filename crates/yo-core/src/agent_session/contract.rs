use crate::{
    ActivityRequestRef, AgentCommand, ApprovalDecision, InputSubmission, SubmissionId,
    SubmissionIdGenerationError, SubmissionRejection, TurnRef, UserInput,
};

/// A frontend intent directed at an agent Session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentIntent {
    /// Starts a Turn or steers the active Turn.
    Submit(InputSubmission),
    /// Steers exactly the Turn observed by the frontend.
    Steer {
        /// The Turn that owned the prompt when the submission was created.
        turn: TurnRef,
        /// The immutable correlated submission.
        submission: InputSubmission,
    },
    /// Requests interruption of the active Turn.
    Interrupt,
    /// Compacts an idle Session with optional user guidance.
    CompactContext { guidance: Option<String> },
    /// Answers one correlated approval request.
    RespondToApproval {
        /// The outstanding request being answered.
        request: ActivityRequestRef,
        /// The user's approval decision.
        decision: ApprovalDecision,
    },
    /// Answers one correlated agent-requested input.
    RespondToUserInput {
        /// The outstanding request being answered.
        request: ActivityRequestRef,
        /// The user's response text.
        input: String,
    },
}

impl AgentIntent {
    /// Captures one plain-text immutable submission with a fresh correlation identity.
    pub fn submit(text: impl Into<String>) -> Result<Self, SubmissionIdGenerationError> {
        Ok(Self::Submit(InputSubmission::new(
            SubmissionId::new()?,
            UserInput::new(text),
        )))
    }
}

/// Immediate result of placing an intent on an agent Session.
#[derive(Debug, Eq, PartialEq)]
pub enum CommandAdmission {
    /// The Session retained the command for delivery.
    Queued,
    /// The Session is busy; the frontend retains and retries this operation.
    Backpressured(PendingCommand),
    /// The correlated submission was rejected before any part reached the worker.
    Rejected {
        /// The immutable submission identity owned by this result.
        id: SubmissionId,
        /// The frontend-neutral reason admission did not occur.
        rejection: SubmissionRejection,
    },
}

/// Result of an admitted Session control that did not mutate durable semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentControlOutcome {
    /// Manual context compaction was unavailable at the current Session boundary.
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

/// An opaque, single-use operation retained across nonblocking backpressure.
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
