//! Crate-private boundary for platform terminal dependencies.

use std::io::Write;

#[cfg(unix)]
pub(crate) mod unix;

pub(crate) trait TerminalBackend {
    type TtyState;
    type Mode: Copy;
    type Error;

    fn capture_tty_state(&mut self) -> Result<Self::TtyState, Self::Error>;
    fn enable_raw_input(&mut self, original: &Self::TtyState) -> Result<(), Self::Error>;
    fn acquire_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error>;
    fn release_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error>;
    fn restore_tty_state(&mut self, state: &Self::TtyState) -> Result<(), Self::Error>;
}

pub(crate) trait ScreenModeBackend: TerminalBackend {
    fn alternate_screen_mode() -> Self::Mode;
    fn cursor_visibility_mode() -> Self::Mode;
}

pub(crate) trait TerminalOutputBackend: TerminalBackend {
    type Output: UnbufferedTerminalOutput;

    fn output(&mut self) -> &mut Self::Output;
}

/// A terminal sink whose successful `write` count is the exact downstream byte prefix and whose
/// `flush` implementation never transfers bytes hidden above that count.
pub(crate) trait UnbufferedTerminalOutput: Write {}

impl UnbufferedTerminalOutput for Vec<u8> {}

impl<Output> UnbufferedTerminalOutput for &mut Output where Output: UnbufferedTerminalOutput + ?Sized
{}
