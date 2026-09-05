//! Shared transactional lifecycle for Inline and Fullscreen modes.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the lifecycle core lands before its Unix adapter consumer"
    )
)]

pub(crate) mod fullscreen;
pub(crate) mod inline;
pub(crate) mod panic_route;
pub(crate) mod screen;
mod transaction;
pub(crate) use transaction::{
    CleanupFailure, CleanupFailures, SessionFailure, SessionFailureCause, TerminalSession,
};
#[cfg(test)]
pub(crate) use transaction::{CleanupFailureCause, CleanupStep};

#[cfg(test)]
mod tests;
