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
    /// Returns to the preceding question while retaining the current unsubmitted draft.
    PreviousQuestion {
        /// Optional one-based selected option; absent means a free-text draft.
        choice: Option<u32>,
        /// Unsubmitted text or notes, preserved independently of any selection.
        draft: UserInput,
    },
    Approval(ApprovalDecision),
    UserInput(UserInput),
    /// One selected option with independently preserved optional notes.
    QuestionAnswer {
        /// One-based ordinal within the outstanding question's choices.
        choice: u32,
        /// Additional user text; an empty value submits the choice alone.
        notes: UserInput,
    },
}

impl ActivityResponse {
    pub(crate) fn has_images(&self) -> bool {
        match self {
            Self::Approval(_) => false,
            Self::UserInput(input)
            | Self::QuestionAnswer { notes: input, .. }
            | Self::PreviousQuestion { draft: input, .. } => !input.images().is_empty(),
        }
    }

    pub(crate) fn has_resolved_skill(&self) -> bool {
        match self {
            Self::Approval(_) => false,
            Self::UserInput(input)
            | Self::QuestionAnswer { notes: input, .. }
            | Self::PreviousQuestion { draft: input, .. } => input.resolved_skill().is_some(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalDecision {
    Approved,
    Declined,
    /// One-based choice in the exact outstanding approval request; the backend owns its scope.
    Offered(u32),
}
