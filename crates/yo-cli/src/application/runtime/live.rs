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
    execution::process,
    interaction::diagnostic::AppError,
    state::{config, connection},
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
                    storage.reader().map(|reader| {
                        reader as &dyn yo_core::session_repository::StoredSessionReader
                    }),
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
    read_only_storage: Option<&crate::state::storage::LocalReadStorage>,
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
                *live = Some(frontend::build_live_session(*prepared, config, &options));
            },
        }
    }

    let config = snapshots.config;
    frontend::run_terminal_generation(
        termination,
        live,
        config,
        snapshots.credentials,
        snapshots.codex_warnings,
        options,
    )
}

fn domain_selection(selection: command::LiveSelection) -> live::LiveSelection {
    match selection {
        command::LiveSelection::New => live::LiveSelection::New,
        command::LiveSelection::Resume(session_id) => live::LiveSelection::Resume(session_id),
        command::LiveSelection::Continue => live::LiveSelection::Continue,
    }
}

fn read_only_resume_output(
    reader: Option<&dyn yo_core::session_repository::StoredSessionReader>,
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
