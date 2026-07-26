//! Typed terminal effects before platform byte encoding.

mod ops;

pub use ops::{TerminalOp, TerminalOps};

#[cfg(test)]
mod tests;
