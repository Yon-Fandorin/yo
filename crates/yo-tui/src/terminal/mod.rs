//! Typed terminal effects before platform byte encoding.

mod ansi;
pub(crate) mod backend;
pub(crate) mod mode;
mod ops;

pub use ansi::{AnsiEncodeError, AnsiEncoder};
pub use ops::{TerminalOp, TerminalOps};

#[cfg(test)]
mod tests;
