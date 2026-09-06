#![cfg(unix)]

mod agent;
mod application;
mod command;
mod connection;
mod diagnostic;
mod execution;
mod live;
mod presentation;
mod print;
mod process;
mod state;

pub(crate) use diagnostic::AppError;
// Temporary routes for consumers moved by the next ownership patch.
pub(crate) use execution::{host, model, tools as local_tools};
pub(crate) use state::{config, storage};

pub fn run() -> std::process::ExitCode {
    application::run()
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) use application::write_session_output;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod pty_tests;
