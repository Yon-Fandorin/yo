//! Knowledge, review, trusted-approval, and Checkpoint proposal tooling for the
//! Methexis SOT Pilot in `yo`.
//!
//! Source validation remains closed until its owning Slice is accepted, so a
//! trusted active-Checkpoint proposal cannot yet make knowledge eligible.

mod check;
mod checkpoint;
mod cli;
mod model;
mod publication;
mod review;

use std::path::Path;

pub use check::{CheckReport, Diagnostic, DiagnosticPhase, UnitRevision};
pub use cli::run;

/// Checks the tracked Draft corpus rooted at `repository_root`.
#[must_use]
pub fn check_repository(repository_root: &Path) -> CheckReport {
    check::check_repository(repository_root)
}
