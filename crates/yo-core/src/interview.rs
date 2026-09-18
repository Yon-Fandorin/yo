//! Public/redacted interview capture and editable copies, independent of backend wire state.
//! Secret values remain outside these durable profiles.

use std::{
    error::Error,
    fmt::{Display, Formatter, Result},
    io::Error as IoError,
};

mod capture;
mod profile;
mod refs;
mod repository;
#[cfg(test)]
mod tests;
mod working_copy;

pub(crate) use capture::initial_submission_evidence;
pub use capture::{CapturedInterview, InterviewCatalog};
pub use profile::{
    Answer, AnswerResponse, Capture, InterviewOption, InterviewQuestion, SecretAnswerState,
};
pub use repository::{InterviewCopyEntry, InterviewRepository};
pub use working_copy::{NewConversation, Submission, WorkingCopy};

pub const CAPTURE_LIMIT: usize = 1024 * 1024;
pub const COPY_LIMIT: usize = 256 * 1024;
pub const PREVIEW_LIMIT: usize = 64 * 1024;

/// Storage conflicts and invalid evidence never turn an editable copy into a saved answer.
#[derive(Debug)]
pub enum InterviewError {
    Invalid(String),
    Conflict,
    Busy,
    Io(IoError),
}

impl Display for InterviewError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Conflict => f.write_str(
                "another writer published this interview copy; editable changes are retained",
            ),
            Self::Busy => {
                f.write_str("the interview repository is busy; editable changes are retained")
            },
            Self::Io(error) => error.fmt(f),
        }
    }
}
impl Error for InterviewError {}
impl From<IoError> for InterviewError {
    fn from(value: IoError) -> Self {
        Self::Io(value)
    }
}
pub(super) fn invalid(message: impl Into<String>) -> InterviewError {
    InterviewError::Invalid(message.into())
}

/// Plain diagnostic on a genuine response; it grants no capture or submission authority.
pub const RECOVERY_UNAVAILABLE_RECEIPT_PREFIX: &str =
    "Interview wire answers accepted; complete recovery unavailable:";

/// Maximum retained plain recovery diagnostic in UTF-8 bytes; oversized lines are rejected.
pub const RECOVERY_DIAGNOSTIC_LIMIT: usize = 4096;
