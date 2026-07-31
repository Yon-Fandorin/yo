#[cfg(unix)]
use std::{error::Error, fmt};
use std::{io::Write, process::ExitCode};

#[cfg(unix)]
mod agent;
#[cfg(unix)]
mod process;
#[cfg(unix)]
mod storage;

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
    let options = parse_options(std::env::args_os().skip(1))?;
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Options {
    mode: yo_tui::PresentationMode,
    glyph_profile: yo_tui::GlyphProfile,
}

#[cfg(unix)]
fn run_agent_generation(
    termination: &mut impl yo_tui::TerminationSource,
    live: &mut Option<LiveSession>,
    cwd: &std::path::Path,
    options: Options,
) -> Result<SessionStep, AppError> {
    if live.is_none() {
        let repository = storage::open_default()
            .map_err(|error| AppError::single("opening the Session repository", error))?;
        let session_id = repository
            .next_session_id()
            .map_err(|error| AppError::single("allocating a Session identity", error))?;
        let backend = yo_core::CodexBackend::spawn(yo_core::CodexBackendConfig::new(cwd))
            .map_err(|error| AppError::single("starting Codex", error))?;
        let Some(agent) = agent::TuiAgentConnection::start_persistent(
            backend,
            session_id,
            repository,
            termination,
        )
        .map_err(|error| AppError::single("creating the agent Session", error))?
        else {
            return Ok(SessionStep::Complete);
        };
        *live = Some(LiveSession {
            agent,
            tui: yo_tui::TuiSession::with_glyph_profile(options.glyph_profile),
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
fn parse_options(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<Options, AppError> {
    const USAGE: &str = "yo [--inline | --fullscreen] [--ascii]";

    let mut mode = None;
    let mut glyph_profile = None;
    for argument in arguments {
        let selected_mode = match argument.as_os_str() {
            value if value == "--inline" => Some(yo_tui::PresentationMode::Inline),
            value if value == "--fullscreen" => Some(yo_tui::PresentationMode::Fullscreen),
            value if value == "--ascii" => {
                if glyph_profile.replace(yo_tui::GlyphProfile::Ascii).is_some() {
                    return Err(AppError::many([format!(
                        "duplicate argument `--ascii`; usage: {USAGE}"
                    )]));
                }
                None
            },
            _ => {
                return Err(AppError::many([format!(
                    "unknown argument `{}`; usage: {USAGE}",
                    argument.to_string_lossy()
                )]));
            },
        };
        if let Some(selected_mode) = selected_mode
            && mode.replace(selected_mode).is_some()
        {
            return Err(AppError::many([format!(
                "multiple presentation modes; usage: {USAGE}"
            )]));
        }
    }
    Ok(Options {
        mode: mode.unwrap_or(yo_tui::PresentationMode::Inline),
        glyph_profile: glyph_profile.unwrap_or(yo_tui::GlyphProfile::Rich),
    })
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

#[cfg(all(test, unix))]
mod tests {
    use yo_tui::{GlyphProfile, PresentationMode};

    use super::{Options, parse_options};

    // option이 없는 기존 실행은 Inline 표시와 Rich glyph라는 두 호환 기본값을 함께 유지한다.
    #[test]
    fn no_option_preserves_compatibility_defaults() {
        assert_eq!(
            parse_options([]).unwrap(),
            Options {
                mode: PresentationMode::Inline,
                glyph_profile: GlyphProfile::Rich,
            }
        );
    }

    // 표시 mode와 ASCII glyph option은 순서와 무관하게 terminal 획득 전 하나의 선택으로 해석된다.
    #[test]
    fn explicit_options_select_presentation_and_glyphs() {
        assert_eq!(
            parse_options(["--ascii".into(), "--fullscreen".into()]).unwrap(),
            Options {
                mode: PresentationMode::Fullscreen,
                glyph_profile: GlyphProfile::Ascii,
            }
        );
        assert_eq!(
            parse_options(["--inline".into(), "--ascii".into()]).unwrap(),
            Options {
                mode: PresentationMode::Inline,
                glyph_profile: GlyphProfile::Ascii,
            }
        );
    }

    // 둘 이상의 mode argument는 우선순위를 임의로 정하지 않고 사용법 오류로 거부한다.
    #[test]
    fn multiple_mode_arguments_are_rejected() {
        let error = parse_options(["--inline".into(), "--fullscreen".into()]).unwrap_err();

        assert!(error.to_string().contains("multiple presentation modes"));
    }

    // ASCII option의 중복은 숨은 우선순위 없이 명시적인 사용법 오류로 거부한다.
    #[test]
    fn duplicate_ascii_argument_is_rejected() {
        let error = parse_options(["--ascii".into(), "--ascii".into()]).unwrap_err();

        assert!(error.to_string().contains("duplicate argument `--ascii`"));
    }

    // 알 수 없는 option은 조용히 Inline으로 fallback하지 않고 지원하는 선택지를 안내한다.
    #[test]
    fn unknown_option_is_rejected_without_fallback() {
        let error = parse_options(["--auto".into()]).unwrap_err();

        assert!(error.to_string().contains("unknown argument `--auto`"));
        assert!(
            error
                .to_string()
                .contains("yo [--inline | --fullscreen] [--ascii]")
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod pty_tests;
