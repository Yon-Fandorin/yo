//! Frontend-independent submitted input and typed reference occurrences.

mod identity;
mod projection;
mod reference;
mod submission;

pub use identity::{SubmissionId, SubmissionIdError, SubmissionIdGenerationError};
pub use projection::{skill_reference_projection, workspace_reference_projection};
pub use reference::{InputReference, UserInput, UserInputError};
pub use submission::{
    InputSubmission, SubmissionOutcome, SubmissionRejection, SubmissionRejectionKind,
};

#[cfg(test)]
mod tests;
