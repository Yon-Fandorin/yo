#[cfg(unix)]
use std::{error::Error, fmt};
use std::{io::Write, process::ExitCode};

#[cfg(unix)]
mod agent;
#[cfg(unix)]
mod command;
#[cfg(unix)]
mod config;
#[cfg(unix)]
mod process;
#[cfg(unix)]
mod session;
#[cfg(unix)]
mod storage;
#[cfg(unix)]
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
    let command = command::parse(std::env::args_os().skip(1))?;
    let options = match command {
        command::Command::Session(command) => {
            let output = session::run(command)?;
            write_session_command_output(output)?;
            return Ok(());
        },
        command::Command::Live(options) => options,
    };
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let mut host = process::termination::TerminationCoordinator::install().map_err(|error| {
        AppError::single("installing the process termination coordinator", error)
    })?;
    let mut live = None;
    let mut job_control = process::job_control::JobControl::new();
    let mut failures = Vec::new();
    loop {
        let generation = host.with_active_resource(
            &mut live,
            |termination, live| run_agent_generation(termination, live, &cwd, options),
            shutdown_live_session,
        );
        match generation {
            Ok(Ok(SessionStep::Suspend)) => {
                if let Err(error) = job_control.suspend() {
                    failures.push(format!("suspending the process: {error}"));
                    break;
                }
            },
            Ok(Ok(SessionStep::Complete)) => break,
            Ok(Err(error)) => {
                failures.extend(error.failures);
                break;
            },
            Err(error) => {
                failures.push(format!("process termination session: {error}"));
                break;
            },
        }
    }
    if let Err(error) = shutdown_live_session(&mut live) {
        failures.extend(error.failures);
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
struct LiveSession {
    agent: agent::TuiAgentConnection,
    tui: yo_tui::TuiSession,
}

#[cfg(unix)]
enum SessionStep {
    Suspend,
    Complete,
}

#[cfg(unix)]
fn run_agent_generation(
    termination: &mut impl yo_tui::TerminationSource,
    live: &mut Option<LiveSession>,
    cwd: &std::path::Path,
    options: command::LiveOptions,
) -> Result<SessionStep, AppError> {
    if live.is_none() {
        let storage = storage::open_default()
            .map_err(|error| AppError::single("opening local Yo storage", error))?;
        let (repository, workspace_host_id) = storage.into_parts();
        let workspace_path = yo_core::HostWorkspacePath::normalize_local(cwd)
            .map_err(|error| AppError::single("normalizing the workspace path", error))?;
        let descriptor = yo_core::SessionDescriptor::new(workspace_host_id, workspace_path)
            .map_err(|error| AppError::single("generating a Session descriptor", error))?;
        let workspace_references =
            yo_core::LocalWorkspaceReferenceProvider::start(cwd, workspace_host_id).map_err(
                |error| AppError::single("starting workspace reference discovery", error),
            )?;
        let backend = yo_core::CodexBackend::spawn(yo_core::CodexBackendConfig::new(cwd))
            .map_err(|error| AppError::single("starting Codex", error))?;
        let Some(agent) = agent::TuiAgentConnection::start_persistent(
            backend,
            descriptor,
            repository,
            termination,
        )
        .map_err(|error| AppError::single("creating the agent Session", error))?
        else {
            return Ok(SessionStep::Complete);
        };
        *live = Some(LiveSession {
            agent,
            tui: yo_tui::TuiSession::with_session_info(
                options.glyph_profile,
                yo_tui::TuiSessionInfo::new("codex", compact_workspace_label(cwd)),
            )
            .with_workspace_references(workspace_references),
        });
    }
    let session = live
        .as_mut()
        .expect("live session is initialized before terminal acquisition");
    let terminal = yo_tui::run_session_with_mode(
        termination,
        &mut session.agent,
        &mut session.tui,
        options.mode,
    );

    let mut failures = Vec::new();
    match terminal {
        Ok(yo_tui::TerminalOutcome::SuspendRequested) => return Ok(SessionStep::Suspend),
        Ok(yo_tui::TerminalOutcome::Exited(outcome)) => {
            if let Some(output) = outcome.output()
                && let Err(error) = write_session_output(output)
            {
                failures.push(format!("writing session output: {error}"));
            }
        },
        Ok(_) => failures.push("terminal session: unsupported terminal outcome".to_owned()),
        Err(error) => failures.push(format!("terminal session: {error}")),
    }
    if let Err(error) = shutdown_live_session(live) {
        failures.extend(error.failures);
    }
    if failures.is_empty() {
        Ok(SessionStep::Complete)
    } else {
        Err(AppError::many(failures))
    }
}

#[cfg(unix)]
fn compact_workspace_label(cwd: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    compact_workspace_label_with_home(cwd, home.as_deref())
}

#[cfg(unix)]
fn compact_workspace_label_with_home(
    cwd: &std::path::Path,
    home: Option<&std::path::Path>,
) -> String {
    let Some(home) = home else {
        return cwd.to_string_lossy().into_owned();
    };
    let Ok(relative) = cwd.strip_prefix(home) else {
        return cwd.to_string_lossy().into_owned();
    };
    if relative.as_os_str().is_empty() {
        "~".to_owned()
    } else {
        format!("~/{}", relative.to_string_lossy())
    }
}

#[cfg(all(test, unix))]
mod workspace_label_tests {
    use std::path::Path;

    use super::compact_workspace_label_with_home;

    // 홈 아래 작업공간은 사용자가 익숙한 `~/...` 표기로 줄이되 경로의 나머지는 보존한다.
    #[test]
    fn home_workspace_uses_tilde_without_losing_the_relative_path() {
        assert_eq!(
            compact_workspace_label_with_home(
                Path::new("/home/yon/projects/yo"),
                Some(Path::new("/home/yon")),
            ),
            "~/projects/yo"
        );
        assert_eq!(
            compact_workspace_label_with_home(Path::new("/home/yon"), Some(Path::new("/home/yon"))),
            "~"
        );
    }

    // 홈 밖 경로이거나 홈 정보를 모르는 경우에는 의미가 달라지지 않도록 절대 경로를 유지한다.
    #[test]
    fn external_workspace_remains_an_absolute_path() {
        assert_eq!(
            compact_workspace_label_with_home(
                Path::new("/srv/work/yo"),
                Some(Path::new("/home/yon")),
            ),
            "/srv/work/yo"
        );
        assert_eq!(
            compact_workspace_label_with_home(Path::new("/srv/work/yo"), None),
            "/srv/work/yo"
        );
    }
}

#[cfg(unix)]
fn shutdown_live_session(live: &mut Option<LiveSession>) -> Result<(), AppError> {
    let Some(mut session) = live.take() else {
        return Ok(());
    };
    session
        .agent
        .shutdown()
        .map(drop)
        .map_err(|error| AppError::single("agent cleanup", error))
}

#[cfg(unix)]
fn write_session_output(output: &str) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(output.as_bytes())?;
    stdout.flush()
}

#[cfg(unix)]
fn write_session_command_output(output: session::Output) -> Result<(), AppError> {
    let mut failures = Vec::new();
    if let Err(error) = write_session_output(&output.stdout) {
        failures.push(format!("writing Session command output: {error}"));
    }
    let mut stderr = std::io::stderr().lock();
    for diagnostic in output.diagnostics {
        if let Err(error) = writeln!(stderr, "yo: warning: {diagnostic}") {
            failures.push(format!("writing Session command diagnostic: {error}"));
            break;
        }
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

#[cfg(all(test, target_os = "linux"))]
mod pty_tests;
