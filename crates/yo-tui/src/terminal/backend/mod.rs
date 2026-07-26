//! Crate-private boundary for platform terminal dependencies.

#[cfg(unix)]
mod unix;

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
