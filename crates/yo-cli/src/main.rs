#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
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
mod live;
#[cfg(unix)]
mod local_tools;
#[cfg(unix)]
mod model;
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
    let mut options = match command {
        command::Command::Session(command) => {
            let output = session::run(command)?;
            write_session_command_output(output)?;
            return Ok(());
        },
        command::Command::Live(options) => options,
    };
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let launch_failure_selection =
        match live::prepare(options.selection, &cwd, options.glyph_profile)? {
            live::LivePreparation::New => command::LiveSelection::New,
            live::LivePreparation::Resume {
                session_id,
                failure_selection,
            } => {
                options.selection = command::LiveSelection::Resume(session_id);
                failure_selection
            },
            live::LivePreparation::ReadOnly(output) => {
                write_session_command_output(output)?;
                return Ok(());
            },
        };
    // Live configuration is snapshotted once and retained across terminal ownership generations.
    let config =
        config::load().map_err(|error| AppError::single("reading Yo configuration", error))?;
    let credentials = model::open_credentials(&config.credential_path())?;
    let mut host = process::termination::TerminationCoordinator::install().map_err(|error| {
        AppError::single("installing the process termination coordinator", error)
    })?;
    let mut live = None;
    let mut job_control = process::job_control::JobControl::new();
    let mut failures = Vec::new();
    loop {
        let generation = host.with_active_resource(
            &mut live,
            |termination, live| {
                run_agent_generation(
                    termination,
                    live,
                    &cwd,
                    options.clone(),
                    launch_failure_selection,
                    &config,
                    &credentials,
                )
            },
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
            Ok(Ok(SessionStep::Continue)) => {},
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
    workspace: std::path::PathBuf,
}

#[cfg(unix)]
enum SessionStep {
    Suspend,
    Continue,
    Complete,
}

#[cfg(unix)]
fn run_agent_generation(
    termination: &mut impl yo_tui::TerminationSource,
    live: &mut Option<LiveSession>,
    cwd: &std::path::Path,
    options: command::LiveOptions,
    launch_failure_selection: command::LiveSelection,
    config: &config::Config,
    credentials: &yo_core::CredentialStore,
) -> Result<SessionStep, AppError> {
    if live.is_none() {
        let storage = match storage::open_default() {
            Ok(storage) => storage,
            Err(error) => {
                return handle_launch_failure(
                    launch_failure_selection,
                    options.glyph_profile,
                    live::ResumeFailureStage::WritableStorage,
                    error,
                );
            },
        };
        let (mut repository, workspace_host_id) = storage.into_parts();
        let launch = match options.selection {
            command::LiveSelection::New => {
                let workspace_path = yo_core::HostWorkspacePath::normalize_local(cwd)
                    .map_err(|error| AppError::single("normalizing the workspace path", error))?;
                Launch::New(
                    yo_core::SessionDescriptor::new(workspace_host_id, workspace_path).map_err(
                        |error| AppError::single("generating a Session descriptor", error),
                    )?,
                )
            },
            command::LiveSelection::Resume(session_id) => {
                let continuation =
                    match yo_core::session_repository::recover_stored_session_continuation(
                        &mut repository,
                        session_id,
                    ) {
                        Ok(continuation) => continuation,
                        Err(error) => {
                            drop(repository);
                            return handle_launch_failure(
                                launch_failure_selection,
                                options.glyph_profile,
                                live::ResumeFailureStage::Revalidation,
                                error,
                            );
                        },
                    };
                if continuation.descriptor().workspace_host_id() != workspace_host_id {
                    drop(repository);
                    return handle_launch_failure(
                        launch_failure_selection,
                        options.glyph_profile,
                        live::ResumeFailureStage::Revalidation,
                        "the Session belongs to another workspace host",
                    );
                }
                Launch::Resume(Box::new(continuation))
            },
            command::LiveSelection::Continue => {
                unreachable!("--continue is resolved before the live generation")
            },
        };
        let session_cwd = match &launch {
            Launch::New(_) => cwd.to_owned(),
            Launch::Resume(continuation) => std::path::PathBuf::from(std::ffi::OsStr::from_bytes(
                continuation.descriptor().workspace_path().as_unix_bytes(),
            )),
        };
        if !session_cwd.is_dir() {
            if matches!(&launch, Launch::Resume(_)) {
                drop(repository);
                return handle_launch_failure(
                    launch_failure_selection,
                    options.glyph_profile,
                    live::ResumeFailureStage::RecordedWorkspace,
                    session_cwd.display(),
                );
            }
            return Err(AppError::many([format!(
                "workspace is unavailable at {}",
                session_cwd.display()
            )]));
        }
        let workspace_references = match yo_core::LocalWorkspaceReferenceProvider::start(
            &session_cwd,
            workspace_host_id,
        ) {
            Ok(provider) => provider,
            Err(error) => {
                if launch.resume_id().is_some() {
                    drop(repository);
                    return handle_launch_failure(
                        launch_failure_selection,
                        options.glyph_profile,
                        live::ResumeFailureStage::WorkspaceReferences,
                        error,
                    );
                }
                return Err(AppError::single(
                    "starting workspace reference discovery",
                    error,
                ));
            },
        };
        let selection = match model::resolve(
            config,
            options.model.as_deref(),
            match &launch {
                Launch::New(_) => None,
                Launch::Resume(continuation) => Some(continuation.target()),
            },
        ) {
            Ok(selection) => selection,
            Err(error) if launch.resume_id().is_some() => {
                drop(repository);
                return handle_launch_failure(
                    launch_failure_selection,
                    options.glyph_profile,
                    live::ResumeFailureStage::BackendSpawn,
                    error,
                );
            },
            Err(error) => return Err(error),
        };
        let (backend, skill_references): (
            Box<dyn yo_core::AgentBackend + Send>,
            Option<yo_core::CodexSkillReferenceProvider>,
        ) = match &selection {
            model::StartupBackend::Codex => {
                let codex_config = yo_core::CodexBackendConfig::new(&session_cwd);
                let skills = match yo_core::CodexSkillReferenceProvider::start(
                    codex_config.clone(),
                    workspace_host_id,
                ) {
                    Ok(skills) => skills,
                    Err(error) if launch.resume_id().is_some() => {
                        drop(repository);
                        return handle_launch_failure(
                            launch_failure_selection,
                            options.glyph_profile,
                            live::ResumeFailureStage::SkillReferences,
                            error,
                        );
                    },
                    Err(error) => {
                        return Err(AppError::single("starting Codex skill discovery", error));
                    },
                };
                let backend = match yo_core::CodexBackend::spawn(codex_config) {
                    Ok(backend) => backend,
                    Err(error) if launch.resume_id().is_some() => {
                        drop(repository);
                        return handle_launch_failure(
                            launch_failure_selection,
                            options.glyph_profile,
                            live::ResumeFailureStage::BackendSpawn,
                            error,
                        );
                    },
                    Err(error) => return Err(AppError::single("starting Codex", error)),
                };
                (Box::new(backend), Some(skills))
            },
            model::StartupBackend::Native { .. } => {
                let backend =
                    match model::start_native(config, credentials, &selection, &session_cwd) {
                        Ok(backend) => backend,
                        Err(error) if launch.resume_id().is_some() => {
                            drop(repository);
                            return handle_launch_failure(
                                launch_failure_selection,
                                options.glyph_profile,
                                live::ResumeFailureStage::BackendSpawn,
                                error,
                            );
                        },
                        Err(error) => return Err(error),
                    };
                (backend, None)
            },
        };
        let agent = match launch {
            Launch::New(descriptor) => agent::TuiAgentConnection::start_persistent(
                backend,
                descriptor,
                repository,
                termination,
            ),
            Launch::Resume(continuation) => {
                let replace_binding = selection.replaces_binding();
                match agent::TuiAgentConnection::start_resumed(
                    backend,
                    *continuation,
                    repository,
                    replace_binding,
                    termination,
                ) {
                    Ok(agent) => Ok(agent),
                    Err(error) => {
                        return handle_launch_failure(
                            launch_failure_selection,
                            options.glyph_profile,
                            live::ResumeFailureStage::NativeResume,
                            error,
                        );
                    },
                }
            },
        }
        .map_err(|error| AppError::single("creating the agent Session", error))?;
        let Some(agent) = agent else {
            return Ok(SessionStep::Complete);
        };
        let mut tui = yo_tui::TuiSession::with_session_info(
            options.glyph_profile,
            yo_tui::TuiSessionInfo::new(selection.label(), compact_workspace_label(&session_cwd)),
            terminal_color_capability(),
            yo_tui::MotionPreference::Standard,
        )
        .with_frame_rate_limit(config.frame_rate_limit())
        .with_workspace_references(workspace_references);
        if let Some(skill_references) = skill_references {
            tui = tui.with_skill_references(skill_references);
        }
        if selection.model_selection().is_some() {
            tui = tui.with_model_selection(yo_core::ModelSelectionController::new(
                config.model_catalog().clone(),
                selection.model_selection(),
            ));
        }
        *live = Some(LiveSession {
            agent,
            tui,
            workspace: session_cwd,
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
        Ok(yo_tui::TerminalOutcome::ModelSelectionRequested(selection)) => {
            let replacement = model::replacement(&selection);
            match model::start_native(config, credentials, &replacement, &session.workspace) {
                Ok(backend) => match session.agent.replace_backend(backend, termination) {
                    Ok(outcome) => {
                        let cleanup_warning = outcome.cleanup_failure().map(ToString::to_string);
                        let label = selection.model().to_string();
                        session.tui.commit_model_switch(
                            yo_core::ModelSelectionController::new(
                                config.model_catalog().clone(),
                                Some(selection),
                            ),
                            label,
                            cleanup_warning,
                        );
                        return Ok(SessionStep::Continue);
                    },
                    Err(error) => {
                        session.tui.report_model_switch_failure(error.to_string());
                        return Ok(SessionStep::Continue);
                    },
                },
                Err(error) => {
                    session.tui.report_model_switch_failure(error.to_string());
                    return Ok(SessionStep::Continue);
                },
            }
        },
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
fn complete_with_read_only_resume(
    session_id: yo_core::SessionId,
    glyph_profile: yo_tui::GlyphProfile,
    reason: &str,
) -> Result<SessionStep, AppError> {
    let output = session::resume_read_only(session_id, glyph_profile, reason)?;
    write_session_command_output(output)?;
    Ok(SessionStep::Complete)
}

#[cfg(unix)]
fn handle_launch_failure(
    selection: command::LiveSelection,
    glyph_profile: yo_tui::GlyphProfile,
    stage: live::ResumeFailureStage,
    detail: impl fmt::Display,
) -> Result<SessionStep, AppError> {
    match live::classify_launch_failure(selection, stage, detail) {
        live::ResumeFailureDisposition::Abort(reason) => Err(AppError::many([reason])),
        live::ResumeFailureDisposition::ReadOnly { session_id, reason } => {
            complete_with_read_only_resume(session_id, glyph_profile, &reason)
        },
    }
}

#[cfg(unix)]
enum Launch {
    New(yo_core::SessionDescriptor),
    Resume(Box<yo_core::session_repository::StoredSessionContinuation>),
}

#[cfg(unix)]
impl Launch {
    fn resume_id(&self) -> Option<yo_core::SessionId> {
        match self {
            Self::New(_) => None,
            Self::Resume(continuation) => Some(continuation.descriptor().session_id()),
        }
    }
}

#[cfg(unix)]
fn terminal_color_capability() -> yo_tui::ColorCapability {
    classify_terminal_color_capability(
        std::env::var_os("COLORTERM")
            .as_deref()
            .and_then(std::ffi::OsStr::to_str),
        std::env::var_os("TERM")
            .as_deref()
            .and_then(std::ffi::OsStr::to_str),
        std::env::var_os("NO_COLOR").is_some(),
    )
}

#[cfg(unix)]
fn classify_terminal_color_capability(
    color_term: Option<&str>,
    term: Option<&str>,
    no_color: bool,
) -> yo_tui::ColorCapability {
    if no_color {
        return yo_tui::ColorCapability::Unknown;
    }
    if color_term.is_some_and(|value| {
        value.eq_ignore_ascii_case("truecolor") || value.eq_ignore_ascii_case("24bit")
    }) {
        return yo_tui::ColorCapability::TrueColor;
    }
    if term.is_some_and(|value| value.to_ascii_lowercase().contains("256color")) {
        return yo_tui::ColorCapability::Limited;
    }
    yo_tui::ColorCapability::Unknown
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

#[cfg(all(test, unix))]
mod color_capability_tests {
    use yo_tui::ColorCapability;

    use super::classify_terminal_color_capability;

    // 명시적인 truecolor 표시는 24-bit ramp 사용을 허용하고 대소문자 차이는 의미를 바꾸지 않는다.
    #[test]
    fn explicit_color_term_selects_true_color() {
        assert_eq!(
            classify_terminal_color_capability(Some("TRUECOLOR"), Some("xterm-256color"), false),
            ColorCapability::TrueColor
        );
        assert_eq!(
            classify_terminal_color_capability(Some("24bit"), None, false),
            ColorCapability::TrueColor
        );
    }

    // 256-color TERM만 확인되면 RGB를 과장하지 않고 제한 색상 fallback을 선택한다.
    #[test]
    fn term_256color_selects_the_limited_fallback() {
        assert_eq!(
            classify_terminal_color_capability(None, Some("screen-256color"), false),
            ColorCapability::Limited
        );
    }

    // NO_COLOR 또는 아무 증거가 없는 환경은 RGB를 내보내지 않는 Unknown 경계로 닫는다.
    #[test]
    fn missing_or_suppressed_color_evidence_stays_unknown() {
        assert_eq!(
            classify_terminal_color_capability(Some("truecolor"), None, true),
            ColorCapability::Unknown
        );
        assert_eq!(
            classify_terminal_color_capability(None, None, false),
            ColorCapability::Unknown
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
