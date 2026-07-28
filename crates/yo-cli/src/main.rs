#[cfg(unix)]
use std::{error::Error, fmt};
use std::{io::Write, process::ExitCode};

#[cfg(unix)]
mod process;

#[cfg(unix)]
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(std::io::stderr().lock(), "yo: {error}");
            ExitCode::FAILURE
        },
    }
}

#[cfg(unix)]
fn run() -> Result<(), AppError> {
    let mut host =
        process::termination::TerminationCoordinator::install().map_err(AppError::Host)?;
    let session = host
        .with_active_session(yo_tui::run)
        .map_err(AppError::Host)?;
    let shutdown = host.shutdown();

    match (session, shutdown) {
        (Ok(_), Ok(())) => Ok(()),
        (Err(session), Ok(())) => Err(AppError::Session(session)),
        (Ok(_), Err(shutdown)) => Err(AppError::Host(shutdown)),
        (Err(session), Err(shutdown)) => Err(AppError::SessionAndShutdown { session, shutdown }),
    }
}

#[cfg(unix)]
#[derive(Debug)]
enum AppError {
    Host(process::termination::HostError),
    Session(yo_tui::RunError),
    SessionAndShutdown {
        session: yo_tui::RunError,
        shutdown: process::termination::HostError,
    },
}

#[cfg(unix)]
impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host(error) => error.fmt(formatter),
            Self::Session(error) => error.fmt(formatter),
            Self::SessionAndShutdown { session, shutdown } => {
                write!(
                    formatter,
                    "{session}; additionally, shutdown failed: {shutdown}"
                )
            },
        }
    }
}

#[cfg(unix)]
impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Host(error) => Some(error),
            Self::Session(error) | Self::SessionAndShutdown { session: error, .. } => Some(error),
        }
    }
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    let _ = writeln!(
        std::io::stderr().lock(),
        "yo: this build currently supports macOS and Linux only"
    );
    ExitCode::FAILURE
}
