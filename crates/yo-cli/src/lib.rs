#![cfg(unix)]

mod application;
mod command;
mod execution;
mod interaction;
mod state;

pub(crate) use interaction::diagnostic::AppError;

pub fn run() -> std::process::ExitCode {
    application::run()
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) use application::write_session_output;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod pty_tests;
