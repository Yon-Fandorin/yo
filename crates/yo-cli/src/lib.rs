#![cfg(unix)]

mod account;
mod agent;
mod application;
mod command;
mod config;
mod connection;
mod diagnostic;
mod host;
mod live;
mod local_tools;
mod model;
mod presentation;
mod print;
mod process;
mod session;
mod storage;
mod usage;

pub(crate) use diagnostic::AppError;

pub fn run() -> std::process::ExitCode {
    application::run()
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) use application::write_session_output;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod pty_tests;
