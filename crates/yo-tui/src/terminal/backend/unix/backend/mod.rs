use std::io::{self, Write};

use super::{TermiosDriver, TtyStateAdapter};
use crate::terminal::backend::{ScreenModeBackend, TerminalBackend};

const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UnixMode {
    AlternateScreen,
}

#[derive(Debug)]
pub(super) enum UnixBackendError<TtyError> {
    Tty(TtyError),
    Output(io::Error),
}

pub(super) struct UnixBackend<D, W> {
    tty: TtyStateAdapter<D>,
    output: W,
}

impl<D, W> UnixBackend<D, W>
where
    D: TermiosDriver,
    W: Write,
{
    pub(super) fn new(tty: TtyStateAdapter<D>, output: W) -> Self {
        Self { tty, output }
    }

    fn write_mode(&mut self, bytes: &[u8]) -> Result<(), UnixBackendError<D::Error>> {
        self.output
            .write_all(bytes)
            .and_then(|()| self.output.flush())
            .map_err(UnixBackendError::Output)
    }
}

impl<D, W> TerminalBackend for UnixBackend<D, W>
where
    D: TermiosDriver,
    W: Write,
{
    type TtyState = D::State;
    type Mode = UnixMode;
    type Error = UnixBackendError<D::Error>;

    fn capture_tty_state(&mut self) -> Result<Self::TtyState, Self::Error> {
        self.tty.capture().map_err(UnixBackendError::Tty)
    }

    fn enable_raw_input(&mut self, original: &Self::TtyState) -> Result<(), Self::Error> {
        self.tty.enable_raw(original).map_err(UnixBackendError::Tty)
    }

    fn acquire_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
        match mode {
            UnixMode::AlternateScreen => self.write_mode(ENTER_ALTERNATE_SCREEN),
        }
    }

    fn release_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
        match mode {
            UnixMode::AlternateScreen => self.write_mode(LEAVE_ALTERNATE_SCREEN),
        }
    }

    fn restore_tty_state(&mut self, state: &Self::TtyState) -> Result<(), Self::Error> {
        self.tty.restore(state).map_err(UnixBackendError::Tty)
    }
}

impl<D, W> ScreenModeBackend for UnixBackend<D, W>
where
    D: TermiosDriver,
    W: Write,
{
    fn alternate_screen_mode() -> Self::Mode {
        UnixMode::AlternateScreen
    }
}

#[cfg(test)]
mod tests;
