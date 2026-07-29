//! Knowledge, review, trusted-approval, Checkpoint, and Source freshness tooling
//! for the Methexis SOT Pilot in `yo`.
//!
//! Trusted integration is fixed to local `develop`. Working-tree observations
//! may demote Source-aware eligibility to degraded, but never grant authority.

mod check;
mod checkpoint;
mod cli;
mod context;
mod file_identity;
mod model;
mod publication;
mod review;
mod source;

use std::path::Path;

pub use check::{
    CheckClass, CheckOutcome, CheckReport, CheckStatus, Diagnostic, DiagnosticPhase, UnitRevision,
};
pub use cli::run;

/// Checks the tracked Draft corpus rooted at `repository_root`.
#[must_use]
pub fn check_repository(repository_root: &Path) -> CheckReport {
    check::check_repository(repository_root)
}

/// Runs the requested check classes and their prerequisites.
///
/// An empty selection uses the same all-classes default as `check_repository`.
#[must_use]
pub fn check_repository_selected(repository_root: &Path, requested: &[CheckClass]) -> CheckReport {
    check::check_repository_selected(repository_root, requested)
}
