//! Typed terminal effects before platform byte encoding.

mod ansi;
pub(crate) mod backend;
pub(crate) mod clipboard;
pub(crate) mod graphics;
pub(crate) mod mode;
mod ops;

use std::{io, num};

pub(crate) use ansi::RESET_HYPERLINK;
pub use ansi::{AnsiEncodeError, AnsiEncoder};
use crossterm::terminal::size;
pub use ops::{TerminalOp, TerminalOps};

#[cfg(unix)]
pub fn current_width() -> io::Result<num::NonZeroU16> {
    let (width, _) = size()?;
    num::NonZeroU16::new(width)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "terminal reported zero width"))
}

#[cfg(test)]
mod tests;
