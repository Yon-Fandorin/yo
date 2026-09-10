use super::{ResolvedSkill, SubmissionId, UserInput};

/// Execution-host authority for reference validation and immutable skill assembly.
/// Calls run on the runtime worker, not the UI thread. Validation never loads
/// skill bodies; preparation loads only after every reference has been validated.
pub trait InputAdmissionHost: Send + Sync {
    /// Verifies host preparation evidence for every immutable image occurrence.
    /// A snapshot's public constructor alone grants no live admission authority.
    fn validate_images(&self, input: &UserInput) -> Result<(), SubmissionRejection> {
        if input.images().is_empty() {
            Ok(())
        } else {
            Err(SubmissionRejection::new(
                SubmissionRejectionKind::EnvironmentUnavailable,
                "The execution host has no validated preparation evidence for this image input",
            ))
        }
    }

    /// Checks the immutable input against the Session's current execution environment.
    fn validate(&self, input: &UserInput) -> Result<(), SubmissionRejection>;

    /// Revalidates the whole input, then assembles any selected skill from that same snapshot.
    /// Implementations must validate every reference before loading any body, and return
    /// instructions bound to the original selected identity (not a refreshed catalog generation).
    /// The default supports path validation only; runtime rejects unresolved skill occurrences.
    fn prepare(&self, input: &UserInput) -> Result<Option<ResolvedSkill>, SubmissionRejection> {
        self.validate(input)?;
        Ok(None)
    }
}

/// Why a Session's execution-environment authority cannot be configured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputAdmissionConfigurationError {
    /// An authority was already bound to this live Session.
    AlreadyConfigured,
    /// Input has already entered the live Session's admission boundary.
    InputAlreadySubmitted,
}

impl std::fmt::Display for InputAdmissionConfigurationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AlreadyConfigured => "input admission host is already configured",
            Self::InputAlreadySubmitted => "configure input admission before submitting input",
        })
    }
}

impl std::error::Error for InputAdmissionConfigurationError {}

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
    ImageCapabilityUnknown,
    ImageUnsupported,
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
