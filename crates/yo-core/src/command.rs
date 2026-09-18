use std::{error::Error, fmt};

use crate::{ActivityRequestRef, SessionId, TurnRef, UserInput};

#[cfg(test)]
mod tests;

/// Process-local secret Activity input.
///
/// The inner UTF-8 value is intentionally private, has no serialization
/// implementation, and never appears in `Debug` output.
#[derive(Clone, Eq, PartialEq)]
pub struct SecretInput(String);

impl SecretInput {
    /// Maximum UTF-8 byte length of one live secret answer.
    pub const MAX_BYTES: usize = 64 * 1024;

    /// Creates one bounded process-local secret value without truncation.
    pub fn new(value: impl Into<String>) -> Result<Self, SecretInputError> {
        let value = value.into();
        if value.len() > Self::MAX_BYTES {
            return Err(SecretInputError);
        }
        Ok(Self(value))
    }

    /// Borrows the exact UTF-8 value for the owning backend transport boundary.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Transfers the exact UTF-8 value to the owning backend transport boundary.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for SecretInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretInput([REDACTED])")
    }
}

/// A live secret answer exceeded its fixed per-answer byte limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecretInputError;

impl fmt::Display for SecretInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("secret input exceeds the 64 KiB UTF-8 limit")
    }
}

impl Error for SecretInputError {}

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
    /// Process-local exact secret answer for the owning outstanding request.
    SecretInput(SecretInput),
    /// Payload-free semantic receipt created by the runtime after backend acceptance.
    SecretInputSubmitted,
}

impl ActivityResponse {
    pub(crate) fn has_images(&self) -> bool {
        match self {
            Self::Approval(_) | Self::SecretInput(_) | Self::SecretInputSubmitted => false,
            Self::UserInput(input)
            | Self::QuestionAnswer { notes: input, .. }
            | Self::PreviousQuestion { draft: input, .. } => !input.images().is_empty(),
        }
    }

    pub(crate) fn has_resolved_skill(&self) -> bool {
        match self {
            Self::Approval(_) | Self::SecretInput(_) | Self::SecretInputSubmitted => false,
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
