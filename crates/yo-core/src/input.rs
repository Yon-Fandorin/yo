//! Frontend-independent submitted input and typed reference occurrences.

mod identity;
mod image;
mod image_preparation;
mod projection;
mod reference;
mod submission;

pub use identity::{SubmissionId, SubmissionIdError, SubmissionIdGenerationError};
pub use image::{InputImage, InputImageDisplay};
pub use image_preparation::{
    ImagePreparationHost, ImagePreparationRequest, ImagePreparationUpdate, PreparedImageAttachment,
};
pub use projection::{skill_reference_projection, workspace_reference_projection};
pub use reference::{InputReference, ResolvedSkill, UserInput, UserInputError};
pub use submission::{
    InputAdmissionConfigurationError, InputAdmissionHost, InputSubmission, SubmissionOutcome,
    SubmissionRejection, SubmissionRejectionKind,
};

#[cfg(test)]
mod tests;
