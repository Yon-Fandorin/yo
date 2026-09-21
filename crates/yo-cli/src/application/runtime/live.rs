mod fork;
mod generation;
mod new_session;
mod presentation;
mod resume;

use std::env;

use yo_core::session_repository::StoredSessionReader;

use super::{
    super::{
        codex_diagnostics::{CodexWarningCollector, publish_pending_codex_diagnostics},
        output::write_session_command_output,
    },
    session::{SessionStep, shutdown_live_session},
    startup::StartupSnapshots,
};
use crate::{
    application::live_selection as selection,
    command,
    execution::process,
    interaction::diagnostic::AppError,
    state::{config, connection},
};

pub(in crate::application) fn run_live_session(
    mut options: command::LiveOptions,
) -> Result<(), AppError> {
    let cwd = env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let (launch_failure_selection, read_only_storage) =
        match selection::prepare(domain_selection(options.selection), &cwd)? {
            selection::LivePreparation::New => (selection::LiveSelection::New, None),
            selection::LivePreparation::Resume {
                session_id,
                failure_selection,
                storage,
            } => {
                options.selection = command::LiveSelection::Resume(session_id);
                (failure_selection, Some(storage))
            },
            selection::LivePreparation::ReadOnly {
                session_id,
                reason,
                storage,
            } => {
                let output = presentation::read_only_resume_output(
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
    // Live 설정은 한 번 snapshot하고 terminal ownership generation 사이에 유지한다.
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
                generation::run_generation(
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

fn domain_selection(selection: command::LiveSelection) -> selection::LiveSelection {
    match selection {
        command::LiveSelection::New => selection::LiveSelection::New,
        command::LiveSelection::Resume(session_id) => selection::LiveSelection::Resume(session_id),
        command::LiveSelection::Continue => selection::LiveSelection::Continue,
    }
}

pub(super) fn saved_execution_options(
    mut options: command::LiveOptions,
    selection: command::LiveSelection,
) -> command::LiveOptions {
    options.selection = selection;
    options.model = None;
    options.no_tools = false;
    options.sandbox = None;
    options
}

#[cfg(test)]
mod tests;
