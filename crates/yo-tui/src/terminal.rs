//! Typed terminal effects before platform byte encoding.

mod ansi;
pub(crate) mod backend;
pub(crate) mod mode;
mod ops;

pub use ansi::{AnsiEncodeError, AnsiEncoder};
pub use ops::{TerminalOp, TerminalOps};

#[cfg(unix)]
pub fn current_width() -> std::io::Result<std::num::NonZeroU16> {
    let (width, _) = crossterm::terminal::size()?;
    std::num::NonZeroU16::new(width).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "terminal reported zero width",
        )
    })
}

#[cfg(test)]
mod tests;
