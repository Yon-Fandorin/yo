use yo_core::{
    HostWorkspacePath,
    session_repository::{
        SessionForkLimits, SessionTreeLimits, StoredSessionContinuation, StoredSessionReader,
    },
};
use yo_tui::ForkPickerToken;

use super::{
    super::{
        codex_diagnostics::{CodexWarningCollector, publish_pending_codex_diagnostics},
        output::write_session_command_output,
    },
    LiveSession, SessionStep, StartupFrontend, StartupOutcome, StartupSnapshots, frontend,
    shutdown_live_session, startup,
};
use crate::{
    application::live_selection as live,
    command,
    execution::{process, tools::LocalToolRegistryRevision},
    interaction::diagnostic::AppError,
    state::{config, connection, storage},
};

pub(in crate::application) fn run_live_session(
    mut options: command::LiveOptions,
) -> Result<(), AppError> {
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let (launch_failure_selection, read_only_storage) =
        match live::prepare(domain_selection(options.selection), &cwd)? {
            live::LivePreparation::New => (live::LiveSelection::New, None),
            live::LivePreparation::Resume {
                session_id,
                failure_selection,
                storage,
            } => {
                options.selection = command::LiveSelection::Resume(session_id);
                (failure_selection, Some(storage))
            },
            live::LivePreparation::ReadOnly {
                session_id,
                reason,
                storage,
            } => {
                let output = read_only_resume_output(
                    storage
                        .reader()
                        .map(|reader| reader as &dyn StoredSessionReader),
                    session_id,
                    options.glyph_profile,
                    &reason,
                )?;
                write_session_command_output(output)?;
                return Ok(());
            },
        };
    // Live configuration is snapshotted once and retained across terminal ownership generations.
    let mut config =
        config::load().map_err(|error| AppError::single("reading Yo configuration", error))?;
    let captured_preference = connection::load_startup_connections(&mut config)?;
    let stored_preference = match options.selection {
        command::LiveSelection::New => captured_preference,
        command::LiveSelection::Resume(_) | command::LiveSelection::Continue => None,
    };
    let mut credentials = None;
    let mut host = process::termination::TerminationCoordinator::install().map_err(|error| {
        AppError::single("installing the process termination coordinator", error)
    })?;
    let mut live = None;
    let codex_warnings = CodexWarningCollector::default();
    let mut job_control = process::job_control::JobControl::new();
    let mut errors = Vec::<AppError>::new();
    loop {
        let generation = host.with_active_resource(
            &mut live,
            |termination, live| {
                run_generation(
                    termination,
                    live,
                    &cwd,
                    options.clone(),
                    launch_failure_selection,
                    read_only_storage.as_ref(),
                    &mut StartupSnapshots {
                        config: &config,
                        credentials: &mut credentials,
                        stored_preference: stored_preference.as_ref(),
                        codex_warnings: &codex_warnings,
                    },
                )
            },
            shutdown_live_session,
        );
        if let Err(error) = publish_pending_codex_diagnostics(&codex_warnings) {
            errors.push(error);
            match generation {
                Ok(Ok(_)) => {},
                Ok(Err(error)) => errors.push(error),
                Err(error) => errors.push(AppError::message(format!(
                    "process termination session: {error}"
                ))),
            }
            break;
        }
        match generation {
            Ok(Ok(SessionStep::Suspend)) => {
                if let Err(error) = job_control.suspend() {
                    errors.push(AppError::message(format!(
                        "suspending the process: {error}"
                    )));
                    break;
                }
            },
            Ok(Ok(SessionStep::Complete)) => break,
            Ok(Ok(SessionStep::Continue)) => {},
            Ok(Ok(
                SessionStep::New
                | SessionStep::Fork
                | SessionStep::ForkPicker
                | SessionStep::ForkBoundary { .. }
                | SessionStep::Tree
                | SessionStep::Resume(_),
            )) => {
                unreachable!("Session transitions are prepared inside their generation")
            },
            Ok(Err(error)) => {
                errors.push(error);
                break;
            },
            Err(error) => {
                errors.push(AppError::message(format!(
                    "process termination session: {error}"
                )));
                break;
            },
        }
    }
    if let Err(error) = shutdown_live_session(&mut live) {
        errors.push(error);
    }
    if let Err(error) = publish_pending_codex_diagnostics(&codex_warnings) {
        errors.push(error);
    }
    if let Err(error) = host.shutdown() {
        errors.push(AppError::message(format!(
            "process termination cleanup: {error}"
        )));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::combine(errors))
    }
}

fn run_generation(
    termination: &mut impl yo_tui::TerminationSource,
    live: &mut Option<LiveSession>,
    cwd: &std::path::Path,
    options: command::LiveOptions,
    launch_failure_selection: live::LiveSelection,
    read_only_storage: Option<&storage::LocalReadStorage>,
    snapshots: &mut StartupSnapshots<'_>,
) -> Result<SessionStep, AppError> {
    if live.is_none() {
        let outcome = startup::prepare_agent(
            termination,
            cwd,
            &options,
            launch_failure_selection,
            read_only_storage,
            snapshots,
            StartupFrontend::Terminal,
        )?;
        match outcome {
            StartupOutcome::Complete => return Ok(SessionStep::Complete),
            StartupOutcome::Ready(prepared) => {
                let config = snapshots.config;
                *live = Some(frontend::build_live_session(*prepared, config, &options)?);
            },
        }
    }

    let config = snapshots.config;
    let step = frontend::run_terminal_generation(
        termination,
        live,
        config,
        snapshots.credentials,
        snapshots.codex_warnings,
        options.clone(),
    )?;
    match step {
        SessionStep::New => start_new_session(termination, live, options, snapshots),
        SessionStep::Fork => fork_session(termination, live, options, snapshots, None),
        SessionStep::ForkPicker => {
            let current = live
                .as_mut()
                .expect("fork picker requires an existing Session");
            if let Err(error) = show_fork_picker(current) {
                current.tui.report_fork_failure(error.to_string());
            }
            Ok(SessionStep::Continue)
        },
        SessionStep::ForkBoundary { picker, index } => {
            fork_session(termination, live, options, snapshots, Some((picker, index)))
        },
        SessionStep::Tree => {
            let current = live
                .as_mut()
                .expect("Session tree requires an existing Session");
            if let Err(error) = show_session_tree(current) {
                current.tui.report_session_tree_failure(error.to_string());
            }
            Ok(SessionStep::Continue)
        },
        SessionStep::Resume(target) => {
            resume_session(termination, live, target, options, snapshots)
        },
        other => Ok(other),
    }
}

fn start_new_session(
    termination: &mut impl yo_tui::TerminationSource,
    live: &mut Option<LiveSession>,
    mut options: command::LiveOptions,
    snapshots: &mut StartupSnapshots<'_>,
) -> Result<SessionStep, AppError> {
    let current = live
        .as_mut()
        .expect("new Session requires an existing Session");
    if current
        .active_host
        .as_ref()
        .is_some_and(|host| host.as_str() == yo_core::HostId::CODEX)
        && current.active_host_model.is_none()
    {
        current.tui.report_new_session_failure("the current Codex account/model is unavailable; reconnect before starting another session");
        return Ok(SessionStep::Continue);
    }
    if current.local_tool_registry == Some(LocalToolRegistryRevision::LegacyReadFile) {
        current.tui.report_new_session_failure("this historical tool profile cannot be retained in a new session; launch yo with an explicit current profile");
        return Ok(SessionStep::Continue);
    }
    options.selection = command::LiveSelection::New;
    options.model = None;
    options.no_tools = current.local_tool_registry == Some(LocalToolRegistryRevision::NoTools);
    options.sandbox = current
        .active_host_execution
        .filter(|profile| profile.is_read_only_review())
        .map(|_| command::SandboxMode::ReadOnly);
    let mut selected_snapshots = StartupSnapshots {
        config: snapshots.config,
        credentials: snapshots.credentials,
        stored_preference: Some(&current.startup_target),
        codex_warnings: snapshots.codex_warnings,
    };
    let prepared = startup::prepare_new_agent(
        termination,
        &current.workspace,
        &options,
        &mut selected_snapshots,
        current.active_host_model.as_ref(),
    );
    let candidate = match prepared {
        Ok(StartupOutcome::Ready(prepared)) => {
            match frontend::build_live_session(*prepared, snapshots.config, &options) {
                Ok(candidate) => candidate,
                Err(error) => {
                    current.tui.report_new_session_failure(error.to_string());
                    return Ok(SessionStep::Continue);
                },
            }
        },
        Ok(StartupOutcome::Complete) => return Ok(SessionStep::Continue),
        Err(error) => {
            current.tui.report_new_session_failure(error.to_string());
            return Ok(SessionStep::Continue);
        },
    };
    // Prepare a separate writer/backend first. Failure above leaves the old Session intact.
    let mut previous = live.replace(candidate);
    if let Err(error) = shutdown_live_session(&mut previous) {
        // The new Session is already selected; cleanup failure must remain visible.
        live.as_mut()
            .expect("installed new Session")
            .tui
            .report_new_session_cleanup_failure(error.to_string());
    }
    Ok(SessionStep::Continue)
}

fn fork_session(
    termination: &mut impl yo_tui::TerminationSource,
    live: &mut Option<LiveSession>,
    options: command::LiveOptions,
    snapshots: &mut StartupSnapshots<'_>,
    selected: Option<(ForkPickerToken, usize)>,
) -> Result<SessionStep, AppError> {
    let current = live.as_mut().expect("fork requires an existing Session");
    let parent = current.session_id;
    let source = match selected {
        Some((picker, index)) => capture_historical_fork_source(current, picker, index),
        None => {
            current.fork_catalog = None;
            capture_fork_source(current)
        },
    };
    let source = match source {
        Ok(source) => source,
        Err(error) => {
            current.tui.report_fork_failure(error.to_string());
            return Ok(SessionStep::Continue);
        },
    };
    let options = saved_execution_options(options, command::LiveSelection::New);
    let mut selected_snapshots = StartupSnapshots {
        config: snapshots.config,
        credentials: snapshots.credentials,
        stored_preference: None,
        codex_warnings: snapshots.codex_warnings,
    };
    let prepared = startup::prepare_fork_agent(
        termination,
        &current.workspace,
        &options,
        &mut selected_snapshots,
        source,
    );
    let mut candidate = match prepared {
        Ok(StartupOutcome::Ready(prepared)) => {
            match frontend::build_live_session(*prepared, snapshots.config, &options) {
                Ok(candidate) => candidate,
                Err(error) => {
                    current.tui.report_fork_failure(error.to_string());
                    return Ok(SessionStep::Continue);
                },
            }
        },
        Ok(StartupOutcome::Complete) => return Ok(SessionStep::Continue),
        Err(error) => {
            current.tui.report_fork_failure(error.to_string());
            return Ok(SessionStep::Continue);
        },
    };
    candidate.tui.report_fork_started(parent);
    let mut previous = live.replace(candidate);
    if let Err(error) = shutdown_live_session(&mut previous) {
        live.as_mut()
            .expect("forked Session installed")
            .tui
            .report_fork_cleanup_failure(error.to_string());
    }
    Ok(SessionStep::Continue)
}

fn show_fork_picker(current: &mut LiveSession) -> Result<(), AppError> {
    current.fork_catalog = None;
    let storage = storage::open_default_reader()
        .map_err(|error| AppError::single("reading historical fork points", error))?;
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::message("the current Session has no saved context to fork"))?;
    let catalog = current
        .agent
        .capture_fork_catalog(reader, SessionForkLimits::default())
        .map_err(|error| AppError::single("capturing historical fork points", error))?;
    let picker = current
        .tui
        .show_fork_picker(&catalog)
        .map_err(AppError::message)?;
    current.fork_catalog = Some((picker, catalog));
    Ok(())
}

fn capture_historical_fork_source(
    current: &mut LiveSession,
    picker: ForkPickerToken,
    index: usize,
) -> Result<StoredSessionContinuation, AppError> {
    let (captured_picker, catalog) = current
        .fork_catalog
        .take()
        .ok_or_else(|| AppError::message("fork selection expired; open /fork at again"))?;
    if captured_picker != picker {
        return Err(AppError::message(
            "fork selection belongs to an older picker; open /fork at again",
        ));
    }
    let selection = catalog
        .selection(index)
        .map_err(|error| AppError::single("selecting a captured fork point", error))?;
    current
        .agent
        .prepare_historical_fork_source(&selection)
        .map_err(|error| AppError::single("revalidating the historical fork point", error))
}

fn capture_fork_source(current: &LiveSession) -> Result<StoredSessionContinuation, AppError> {
    let storage = storage::open_default_reader()
        .map_err(|error| AppError::single("reading the fork source", error))?;
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::message("the current Session has no saved context to fork"))?;
    current
        .agent
        .capture_fork_source(reader)
        .map_err(|error| AppError::single("capturing the fork source", error))
}

fn resume_session(
    termination: &mut impl yo_tui::TerminationSource,
    live: &mut Option<LiveSession>,
    target: Option<yo_core::SessionId>,
    options: command::LiveOptions,
    snapshots: &mut StartupSnapshots<'_>,
) -> Result<SessionStep, AppError> {
    let current = live.as_mut().expect("resume requires an existing Session");
    let Some(target) = target else {
        if let Err(error) = show_resume_picker(current, snapshots.config) {
            current.tui.report_resume_failure(error.to_string());
        }
        return Ok(SessionStep::Continue);
    };
    if current.session_id == target {
        current
            .tui
            .report_resume_failure("that session is already open");
        return Ok(SessionStep::Continue);
    }
    let preparation = live::prepare(live::LiveSelection::Resume(target), &current.workspace);
    match preparation {
        Ok(live::LivePreparation::Resume { .. }) => {},
        Ok(live::LivePreparation::ReadOnly { reason, .. }) => {
            current.tui.report_resume_failure(format!(
                "{reason}. Inspect saved history with `yo session {target}`."
            ));
            return Ok(SessionStep::Continue);
        },
        Ok(live::LivePreparation::New) => {
            unreachable!("explicit resume cannot start a new Session")
        },
        Err(error) => {
            current.tui.report_resume_failure(error.to_string());
            return Ok(SessionStep::Continue);
        },
    }
    // Resume is selected solely by its stored binding; current-session overrides cannot leak.
    let options = saved_execution_options(options, command::LiveSelection::Resume(target));
    let mut resumed_snapshots = StartupSnapshots {
        config: snapshots.config,
        credentials: snapshots.credentials,
        stored_preference: None,
        codex_warnings: snapshots.codex_warnings,
    };
    // Continue's failure disposition aborts instead of writing archival stdout and closing
    // the terminal flow. The current live Session stays open and receives the exact error.
    let prepared = startup::prepare_agent(
        termination,
        &current.workspace,
        &options,
        live::LiveSelection::Continue,
        None,
        &mut resumed_snapshots,
        StartupFrontend::Terminal,
    );
    let candidate = match prepared {
        Ok(StartupOutcome::Ready(prepared)) => {
            match frontend::build_live_session(*prepared, snapshots.config, &options) {
                Ok(candidate) => candidate,
                Err(error) => {
                    current.tui.report_resume_failure(error.to_string());
                    return Ok(SessionStep::Continue);
                },
            }
        },
        Ok(StartupOutcome::Complete) => return Ok(SessionStep::Continue),
        Err(error) => {
            current.tui.report_resume_failure(format!(
                "{error}. Inspect saved history with `yo session {target}`."
            ));
            return Ok(SessionStep::Continue);
        },
    };
    let mut previous = live.replace(candidate);
    if let Err(error) = shutdown_live_session(&mut previous) {
        live.as_mut()
            .expect("resumed Session installed")
            .tui
            .report_resume_cleanup_failure(error.to_string());
    }
    Ok(SessionStep::Continue)
}

fn saved_execution_options(
    mut options: command::LiveOptions,
    selection: command::LiveSelection,
) -> command::LiveOptions {
    options.selection = selection;
    options.model = None;
    options.no_tools = false;
    options.sandbox = None;
    options
}

fn show_session_tree(current: &mut LiveSession) -> Result<(), AppError> {
    let storage = storage::open_default_reader()
        .map_err(|error| AppError::single("reading saved sessions", error))?;
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::message("no saved sessions exist"))?;
    let workspace_host = storage
        .workspace_host_id()
        .ok_or_else(|| AppError::message("saved workspace identity is unavailable"))?;
    let workspace = HostWorkspacePath::normalize_local(&current.workspace)
        .map_err(|error| AppError::single("normalizing the current workspace", error))?;
    let tree = reader
        .read_tree(workspace_host, &workspace, SessionTreeLimits::default())
        .map_err(|error| AppError::single("reading the session tree", error))?;
    current
        .tui
        .show_session_tree(&tree, current.session_id)
        .map_err(AppError::message)
}

fn show_resume_picker(current: &mut LiveSession, config: &config::Config) -> Result<(), AppError> {
    use yo_tui::ResumeSessionEntry;
    let storage = storage::open_default_reader()
        .map_err(|error| AppError::single("reading saved sessions", error))?;
    let reader = storage
        .reader()
        .ok_or_else(|| AppError::message("no saved sessions exist"))?;
    let workspace = HostWorkspacePath::normalize_local(&current.workspace)
        .map_err(|error| AppError::single("normalizing the current workspace", error))?;
    let sessions = reader
        .discover()
        .map_err(|error| AppError::single("discovering saved sessions", error))?;
    let date_formatter = config.date_formatter().ok();
    let mut entries = Vec::new();
    let mut has_more = false;
    for saved in sessions {
        let Some(summary) = saved.summary() else {
            continue;
        };
        let descriptor = summary.discovery().descriptor();
        if saved.session_id() == current.session_id
            || Some(descriptor.workspace_host_id()) != storage.workspace_host_id()
            || descriptor.workspace_path() != &workspace
        {
            continue;
        }
        if entries.len() == 64 {
            has_more = true;
            break;
        }
        entries.push(ResumeSessionEntry {
            session_id: saved.session_id(),
            eligibility: saved.continuation_eligibility(),
            updated_label: date_formatter
                .as_ref()
                .and_then(|formatter| {
                    formatter
                        .format_unix_millis(summary.discovery().updated_unix_millis())
                        .ok()
                })
                .unwrap_or_else(|| "unknown update time".to_owned()),
        });
    }
    if entries.is_empty() {
        return Err(AppError::message(
            "no other saved sessions exist in this workspace",
        ));
    }
    current
        .tui
        .show_resume_picker(entries, has_more)
        .map_err(AppError::message)
}

fn domain_selection(selection: command::LiveSelection) -> live::LiveSelection {
    match selection {
        command::LiveSelection::New => live::LiveSelection::New,
        command::LiveSelection::Resume(session_id) => live::LiveSelection::Resume(session_id),
        command::LiveSelection::Continue => live::LiveSelection::Continue,
    }
}

fn read_only_resume_output(
    reader: Option<&dyn StoredSessionReader>,
    session_id: yo_core::SessionId,
    glyph_profile: yo_tui::GlyphProfile,
    reason: &str,
) -> Result<command::SessionOutput, AppError> {
    let reader = reader
        .ok_or_else(|| AppError::many([format!("stored Session {session_id} was not found")]))?;
    command::read_only_resume_from(reader, session_id, glyph_profile, reason)
}

#[cfg(test)]
mod tests;
