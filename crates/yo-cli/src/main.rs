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
    let mode = parse_presentation_mode(std::env::args_os().skip(1))?;
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let mut host = process::termination::TerminationCoordinator::install().map_err(|error| {
        AppError::single("installing the process termination coordinator", error)
    })?;
    let session = host.with_active_session(|termination| run_agent_session(termination, cwd, mode));

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
    mode: yo_tui::PresentationMode,
) -> Result<(), AppError> {
    let backend = yo_core::CodexBackend::spawn(yo_core::CodexBackendConfig::new(cwd))
        .map_err(|error| AppError::single("starting Codex", error))?;
    let Some(mut agent) = agent::TuiAgentConnection::start(backend, termination)
        .map_err(|error| AppError::single("creating the agent Session", error))?
    else {
        return Ok(());
    };
    let terminal = yo_tui::run_with_mode(termination, &mut agent, mode);
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
fn parse_presentation_mode(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<yo_tui::PresentationMode, AppError> {
    let mut arguments = arguments.into_iter();
    let mode = match arguments.next().as_deref() {
        None => yo_tui::PresentationMode::Inline,
        Some(argument) if argument == "--inline" => yo_tui::PresentationMode::Inline,
        Some(argument) if argument == "--fullscreen" => yo_tui::PresentationMode::Fullscreen,
        Some(argument) => {
            return Err(AppError::many([format!(
                "unknown argument `{}`; usage: yo [--inline | --fullscreen]",
                argument.to_string_lossy()
            )]));
        },
    };
    if let Some(argument) = arguments.next() {
        return Err(AppError::many([format!(
            "unexpected argument `{}`; usage: yo [--inline | --fullscreen]",
            argument.to_string_lossy()
        )]));
    }
    Ok(mode)
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
    use yo_tui::PresentationMode;

    use super::parse_presentation_mode;

    // option이 없는 기존 실행은 Auto로 추측하지 않고 임시 호환 정책인 Inline을 선택한다.
    #[test]
    fn no_option_preserves_inline_compatibility() {
        assert_eq!(
            parse_presentation_mode([]).unwrap(),
            PresentationMode::Inline
        );
    }

    // 명시적인 두 option은 terminal을 획득하기 전에 대응 presenter를 정확히 선택한다.
    #[test]
    fn explicit_options_select_the_requested_presenter() {
        assert_eq!(
            parse_presentation_mode(["--inline".into()]).unwrap(),
            PresentationMode::Inline
        );
        assert_eq!(
            parse_presentation_mode(["--fullscreen".into()]).unwrap(),
            PresentationMode::Fullscreen
        );
    }

    // 둘 이상의 mode argument는 우선순위를 임의로 정하지 않고 사용법 오류로 거부한다.
    #[test]
    fn multiple_mode_arguments_are_rejected() {
        let error =
            parse_presentation_mode(["--inline".into(), "--fullscreen".into()]).unwrap_err();

        assert!(error.to_string().contains("unexpected argument"));
    }

    // 알 수 없는 option은 조용히 Inline으로 fallback하지 않고 지원하는 선택지를 안내한다.
    #[test]
    fn unknown_option_is_rejected_without_fallback() {
        let error = parse_presentation_mode(["--auto".into()]).unwrap_err();

        assert!(error.to_string().contains("unknown argument `--auto`"));
        assert!(error.to_string().contains("yo [--inline | --fullscreen]"));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod pty_tests;
