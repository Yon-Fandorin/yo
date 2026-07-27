//! Unix TTY state capture and restoration through Rustix.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the termios adapter lands before the composed Unix backend"
    )
)]

use rustix::{
    fd::BorrowedFd,
    io, stdio,
    termios::{OptionalActions, Termios, tcgetattr, tcsetattr},
};

mod backend;
mod input;

pub(super) trait TermiosDriver {
    type State: Clone;
    type Error;

    fn capture(&mut self) -> Result<Self::State, Self::Error>;
    fn make_raw(&mut self, state: &mut Self::State);
    fn apply(&mut self, state: &Self::State) -> Result<(), Self::Error>;
}

pub(super) struct TtyStateAdapter<D> {
    driver: D,
}

impl<D> TtyStateAdapter<D>
where
    D: TermiosDriver,
{
    pub(super) fn new(driver: D) -> Self {
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

pub(super) struct RustixTermiosDriver {
    input: BorrowedFd<'static>,
}

impl RustixTermiosDriver {
    pub(super) fn stdin() -> Self {
        Self {
            input: stdio::stdin(),
        }
    }
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
