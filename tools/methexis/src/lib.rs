//! Draft knowledge validation for the Methexis SOT Pilot in `yo`.
//!
//! Approval, Checkpoint, Source validation, and projection behavior remain
//! absent until their owning Slices are accepted.

mod check;
mod cli;
mod model;

use std::path::Path;

pub use check::{CheckReport, Diagnostic, DiagnosticPhase, UnitRevision};
pub use cli::run;

/// Checks the tracked Draft corpus rooted at `repository_root`.
#[must_use]
pub fn check_repository(repository_root: &Path) -> CheckReport {
    check::check_repository(repository_root)
}
