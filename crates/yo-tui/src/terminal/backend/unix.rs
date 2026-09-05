//! Unix TTY state capture and restoration through Rustix.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the termios adapter lands before the composed Unix backend"
    )
)]

use std::io::{self as std_io, Write};

use rustix::{
    fd::BorrowedFd,
    io, stdio,
    termios::{OptionalActions, Termios, tcgetattr, tcsetattr},
};

mod backend;
mod event;
mod input;

pub(crate) use backend::{UnixBackend, UnixBackendError, UnixMode};
pub(crate) use event::UnixEventReader;
pub(crate) use input::{CrosstermEventSource, EventSource};

pub(crate) trait TermiosDriver {
    type State: Clone;
    type Error;

    fn capture(&mut self) -> Result<Self::State, Self::Error>;
    fn make_raw(&mut self, state: &mut Self::State);
    fn apply(&mut self, state: &Self::State) -> Result<(), Self::Error>;
}

pub(crate) struct TtyStateAdapter<D> {
    driver: D,
}

impl<D> TtyStateAdapter<D>
where
    D: TermiosDriver,
{
    pub(crate) fn new(driver: D) -> Self {
        Self { driver }
    }

    pub(super) fn capture(&mut self) -> Result<D::State, D::Error> {
        self.driver.capture()
    }

    pub(super) fn enable_raw(&mut self, original: &D::State) -> Result<(), D::Error> {
        let mut raw = original.clone();
        self.driver.make_raw(&mut raw);
        self.driver.apply(&raw)
    }

    pub(super) fn restore(&mut self, original: &D::State) -> Result<(), D::Error> {
        self.driver.apply(original)
    }
}

pub(crate) struct RustixTermiosDriver {
    input: BorrowedFd<'static>,
}

pub(crate) struct DirectTerminalWriter {
    output: BorrowedFd<'static>,
}

impl DirectTerminalWriter {
    pub(crate) fn stdout() -> Self {
        Self {
            output: stdio::stdout(),
        }
    }
}

impl Write for DirectTerminalWriter {
    fn write(&mut self, bytes: &[u8]) -> std_io::Result<usize> {
        io::write(self.output, bytes)
            .map_err(|error| std_io::Error::from_raw_os_error(error.raw_os_error()))
    }

    fn flush(&mut self) -> std_io::Result<()> {
        Ok(())
    }
}

impl crate::terminal::backend::UnbufferedTerminalOutput for DirectTerminalWriter {}

impl RustixTermiosDriver {
    pub(crate) fn stdin() -> Self {
        Self {
            input: stdio::stdin(),
        }
    }
}

pub(crate) fn terminal_size() -> Result<crate::surface::Size, io::Errno> {
    let size = rustix::termios::tcgetwinsize(stdio::stdout())?;
    Ok(crate::surface::Size::new(size.ws_col, size.ws_row))
}

impl TermiosDriver for RustixTermiosDriver {
    type State = Termios;
    type Error = io::Errno;

    fn capture(&mut self) -> Result<Self::State, Self::Error> {
        tcgetattr(self.input)
    }

    fn make_raw(&mut self, state: &mut Self::State) {
        state.make_raw();
    }

    fn apply(&mut self, state: &Self::State) -> Result<(), Self::Error> {
        tcsetattr(self.input, OptionalActions::Now, state)
    }
}

#[cfg(test)]
mod tests;
