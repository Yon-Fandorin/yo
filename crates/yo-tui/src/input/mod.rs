//! Semantic input after terminal protocol decoding.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the semantic model lands before its terminal decoder consumer"
    )
)]

pub(crate) mod buffer;
pub(crate) mod control;
pub(crate) mod editor;
pub(crate) mod event;

#[cfg(test)]
mod tests;
