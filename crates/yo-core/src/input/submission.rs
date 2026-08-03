use super::{SubmissionId, UserInput};

/// One immutable frontend snapshot retained until admission resolves.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputSubmission {
    id: SubmissionId,
    input: UserInput,
}

/// Final whole-request admission result correlated to one immutable snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmissionOutcome {
    Accepted {
        id: SubmissionId,
    },
    Rejected {
        id: SubmissionId,
        rejection: SubmissionRejection,
    },
}

/// A frontend-neutral reason why no part of a submission was dispatched.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmissionRejection {
    kind: SubmissionRejectionKind,
    message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SubmissionRejectionKind {
    InvalidReference,
    EnvironmentUnavailable,
    StaleReference,
    Unauthorized,
    Incompatible,
    RequiredAssetUnavailable,
    OverBudget,
    TargetChanged,
}

impl InputSubmission {
    #[must_use]
    pub const fn new(id: SubmissionId, input: UserInput) -> Self {
        Self { id, input }
    }

    #[must_use]
    pub const fn id(&self) -> SubmissionId {
        self.id
    }

    #[must_use]
    pub const fn input(&self) -> &UserInput {
        &self.input
    }

    #[must_use]
    pub fn into_input(self) -> UserInput {
        self.input
    }
}

impl SubmissionOutcome {
    #[must_use]
    pub const fn id(&self) -> SubmissionId {
        match self {
            Self::Accepted { id } | Self::Rejected { id, .. } => *id,
        }
    }
}

impl SubmissionRejection {
    #[must_use]
    pub fn new(kind: SubmissionRejectionKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> SubmissionRejectionKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}
