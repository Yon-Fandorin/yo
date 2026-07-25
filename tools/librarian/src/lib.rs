//! Advisory working-tree Knowledge discovery for the Methexis Pilot.
//!
//! Librarian proposes deterministic candidates and reasons. It does not read
//! or interpret approval, eligibility, or active-Checkpoint authority.

mod catalog;
mod cli;
mod discovery;
mod error;
mod hash;
#[cfg(test)]
mod test_support;
mod wire;

pub use cli::run;
