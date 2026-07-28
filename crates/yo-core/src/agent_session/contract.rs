use crate::{ActivityRequestRef, AgentCommand, ApprovalDecision};

/// A frontend intent directed at an agent Session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentIntent {
    /// Starts a Turn or steers the active Turn.
    Submit(String),
    /// Requests interruption of the active Turn.
    Interrupt,
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

/// Immediate result of placing an intent on an agent Session.
#[derive(Debug, Eq, PartialEq)]
pub enum CommandAdmission {
    /// The Session retained the command for delivery.
    Accepted,
    /// The Session is busy; the frontend retains and retries this operation.
    Backpressured(PendingCommand),
}

/// An opaque, single-use operation retained across nonblocking backpressure.
#[derive(Debug, Eq, PartialEq)]
pub struct PendingCommand(AgentCommand);

impl PendingCommand {
    pub(super) fn from_command(command: AgentCommand) -> Self {
        Self(command)
    }

    pub(super) fn into_command(self) -> AgentCommand {
        self.0
    }

    #[cfg(test)]
    pub(super) const fn command(&self) -> &AgentCommand {
        &self.0
    }
}
