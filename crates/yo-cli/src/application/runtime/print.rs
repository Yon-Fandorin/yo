use super::{
    super::{
        codex_diagnostics::{
            CodexWarningCollector, error_after_codex_diagnostics, publish_pending_codex_diagnostics,
        },
        output::write_command_output,
    },
    PreparedAgent, StartupFrontend, StartupOutcome, StartupSnapshots, shutdown_live_session,
    startup,
};
use crate::{
    command, config, connection, diagnostic::AppError, print as print_projection, process,
};

pub(in crate::application) fn run_print_session(
    options: command::PrintOptions,
) -> Result<(), AppError> {
    let input = print_projection::read_input(options.prompt)?;
    let cwd = std::env::current_dir()
        .map_err(|error| AppError::single("reading the working directory", error))?;
    let mut config =
        config::load().map_err(|error| AppError::single("reading Yo configuration", error))?;
    let stored_preference = connection::load_startup_connections(&mut config)?;
    let mut credentials = None;
    let mut host = process::termination::TerminationCoordinator::install().map_err(|error| {
        AppError::single("installing the process termination coordinator", error)
    })?;
    let mut live = None;
    let codex_warnings = CodexWarningCollector::default();
    let startup = command::LiveOptions {
        mode: yo_tui::PresentationMode::Inline,
        glyph_profile: yo_tui::GlyphProfile::Rich,
        selection: options.selection,
        model: options.model,
        no_tools: options.no_tools,
        sandbox: options.sandbox,
    };
    let generation = host.with_active_resource(
        &mut live,
        |termination, _live| {
            let outcome = startup::prepare_agent(
                termination,
                &cwd,
                &startup,
                command::LiveSelection::New,
                &mut StartupSnapshots {
                    config: &config,
                    credentials: &mut credentials,
                    stored_preference: stored_preference.as_ref(),
                    codex_warnings: &codex_warnings,
                },
                StartupFrontend::Print,
            )?;
            let StartupOutcome::Ready(prepared) = outcome else {
                return Ok(None);
            };
            let PreparedAgent { agent, .. } = *prepared;
            let mut session = agent.into_session();
            let output =
                print_projection::run(&mut session, input, || termination_requested(termination));
            let cleanup = session
                .shutdown()
                .map(drop)
                .map_err(|error| AppError::single("agent cleanup", error));
            match (output, cleanup) {
                (Ok(output), Ok(())) => Ok(Some(output)),
                (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
                (Err(primary), Err(cleanup)) => Err(AppError::combine(vec![primary, cleanup])),
            }
        },
        shutdown_live_session,
    );

    let mut output = None;
    let mut errors = Vec::new();
    match generation {
        Ok(Ok(value)) => output = value,
        Ok(Err(error)) => errors.push(error),
        Err(error) => errors.push(AppError::message(format!(
            "process termination session: {error}"
        ))),
    }
    if let Err(error) = shutdown_live_session(&mut live) {
        errors.push(error);
    }
    if let Err(error) = host.shutdown() {
        errors.push(AppError::message(format!(
            "process termination cleanup: {error}"
        )));
    }
    finish_print_output_with_codex_diagnostics(output, errors, &codex_warnings)
}

fn finish_print_output_with_codex_diagnostics(
    output: Option<String>,
    errors: Vec<AppError>,
    codex_warnings: &CodexWarningCollector,
) -> Result<(), AppError> {
    if !errors.is_empty() {
        return Err(error_after_codex_diagnostics(
            AppError::combine(errors),
            codex_warnings,
        ));
    }
    let output = match output {
        Some(output) => output,
        None => {
            return Err(error_after_codex_diagnostics(
                AppError::message("print session completed without buffered final-response output"),
                codex_warnings,
            ));
        },
    };
    finish_print_output(Some(output), Vec::new(), |output| {
        write_command_output(output)?;
        publish_pending_codex_diagnostics(codex_warnings)
    })
}

fn finish_print_output(
    output: Option<String>,
    errors: Vec<AppError>,
    publish: impl FnOnce(String) -> Result<(), AppError>,
) -> Result<(), AppError> {
    if !errors.is_empty() {
        return Err(AppError::combine(errors));
    }
    let output = output.ok_or_else(|| {
        AppError::message("print session completed without buffered final-response output")
    })?;
    publish(output)
}

fn termination_requested(termination: &mut impl yo_tui::TerminationSource) -> bool {
    use std::task::{Context, Poll};

    let waker = std::task::Waker::noop();
    let mut context = Context::from_waker(waker);
    termination.poll_termination(&mut context) == Poll::Ready(yo_tui::TerminationEvent::Requested)
}

#[cfg(test)]
mod tests;
