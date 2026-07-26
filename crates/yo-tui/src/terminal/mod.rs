//! Typed terminal effects before platform byte encoding.

mod ansi;
mod ops;

pub use ansi::{AnsiEncodeError, AnsiEncoder};
pub use ops::{TerminalOp, TerminalOps};

#[cfg(test)]
mod tests;
