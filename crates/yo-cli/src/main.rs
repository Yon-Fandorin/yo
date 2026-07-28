#[cfg(unix)]
use std::{error::Error, fmt};
use std::{io::Write, process::ExitCode};

#[cfg(unix)]
mod agent;
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
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let mut host = process::termination::TerminationCoordinator::install().map_err(|error| {
        AppError::single("installing the process termination coordinator", error)
    })?;
    let session = host.with_active_session(|termination| run_agent_session(termination, cwd));

    let mut failures = Vec::new();
    match session {
        Ok(Ok(_)) => {},
        Ok(Err(error)) => failures.extend(error.failures),
        Err(error) => failures.push(format!("process termination session: {error}")),
    }
    if let Err(error) = host.shutdown() {
        failures.push(format!("process termination cleanup: {error}"));
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::many(failures))
    }
}

#[cfg(unix)]
fn run_agent_session(
    termination: &mut impl yo_tui::TerminationSource,
    cwd: std::path::PathBuf,
) -> Result<(), AppError> {
    let backend = yo_core::CodexBackend::spawn(yo_core::CodexBackendConfig::new(cwd))
        .map_err(|error| AppError::single("starting Codex", error))?;
    let Some(mut agent) = agent::TuiAgentConnection::start(backend, termination)
        .map_err(|error| AppError::single("creating the agent Session", error))?
    else {
        return Ok(());
    };
    let terminal = yo_tui::run(termination, &mut agent);
    let cleanup = agent.shutdown();

    let mut failures = Vec::new();
    if let Err(error) = terminal {
        failures.push(format!("terminal session: {error}"));
    }
    if let Err(error) = cleanup {
        failures.push(format!("agent cleanup: {error}"));
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::many(failures))
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct AppError {
    failures: Vec<String>,
}

#[cfg(unix)]
impl AppError {
    fn single(context: &'static str, error: impl fmt::Display) -> Self {
        Self::many([format!("{context}: {error}")])
    }

    fn many(failures: impl IntoIterator<Item = String>) -> Self {
        Self {
            failures: failures.into_iter().collect(),
        }
    }
}

#[cfg(unix)]
impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.failures.join("; additionally, "))
    }
}

#[cfg(unix)]
impl Error for AppError {}

#[cfg(not(unix))]
fn main() -> ExitCode {
    let _ = writeln!(
        std::io::stderr().lock(),
        "yo: this build currently supports macOS and Linux only"
    );
    ExitCode::FAILURE
}
