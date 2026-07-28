use std::io::{self, Write};

use super::{TermiosDriver, TtyStateAdapter};
use crate::terminal::backend::{ScreenModeBackend, TerminalBackend, TerminalOutputBackend};

const ENTER_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049h";
const LEAVE_ALTERNATE_SCREEN: &[u8] = b"\x1b[?1049l";
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnixMode {
    AlternateScreen,
    CursorVisibility,
}

#[derive(Debug)]
pub(crate) enum UnixBackendError<TtyError> {
    Tty(TtyError),
    Output(io::Error),
}

pub(crate) struct UnixBackend<D, W> {
    tty: TtyStateAdapter<D>,
    output: W,
}

impl<D, W> UnixBackend<D, W>
where
    D: TermiosDriver,
    W: Write,
{
    pub(crate) fn new(tty: TtyStateAdapter<D>, output: W) -> Self {
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
            UnixMode::CursorVisibility => self.write_mode(SHOW_CURSOR),
        }
    }

    fn release_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error> {
        match mode {
            UnixMode::AlternateScreen => self.write_mode(LEAVE_ALTERNATE_SCREEN),
            UnixMode::CursorVisibility => self.write_mode(SHOW_CURSOR),
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

    fn cursor_visibility_mode() -> Self::Mode {
        UnixMode::CursorVisibility
    }
}

impl<D, W> TerminalOutputBackend for UnixBackend<D, W>
where
    D: TermiosDriver,
    W: Write,
{
    type Output = W;

    fn output(&mut self) -> &mut Self::Output {
        &mut self.output
    }
}

#[cfg(test)]
mod tests;
