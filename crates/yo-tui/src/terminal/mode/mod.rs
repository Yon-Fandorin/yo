//! Shared transactional lifecycle for Inline and Fullscreen modes.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the lifecycle core lands before its Unix adapter consumer"
    )
)]

mod transaction;

#[cfg(test)]
mod tests;
