//! Draft knowledge, review Projection, and approval proposal tooling for
//! the Methexis SOT Pilot in `yo`.
//!
//! Trusted-ref authority, Checkpoint, and Source validation remain absent until
//! their owning Slices are accepted.

mod check;
mod cli;
mod model;
mod review;

use std::path::Path;

pub use check::{CheckReport, Diagnostic, DiagnosticPhase, UnitRevision};
pub use cli::run;

/// Checks the tracked Draft corpus rooted at `repository_root`.
#[must_use]
pub fn check_repository(repository_root: &Path) -> CheckReport {
    check::check_repository(repository_root)
}
