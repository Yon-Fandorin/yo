//! Shared transactional lifecycle for Inline and Fullscreen modes.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the lifecycle core lands before its Unix adapter consumer"
    )
)]

pub(crate) mod screen;
mod transaction;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use transaction::TerminalSession;
