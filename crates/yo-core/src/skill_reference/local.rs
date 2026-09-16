//! Explicit local skill sources shared by execution hosts, without backend policy.

mod admission;
mod catalog;
mod provider;
mod roots;
mod snapshot;
#[cfg(test)]
mod tests;

pub use admission::LocalSkillInputAdmission;
pub use provider::LocalSkillReferenceProvider;
pub use roots::LocalSkillRoot;

use crate::{SubmissionRejection, SubmissionRejectionKind};

fn reject(kind: SubmissionRejectionKind, message: impl Into<String>) -> SubmissionRejection {
    SubmissionRejection::new(kind, message)
}
