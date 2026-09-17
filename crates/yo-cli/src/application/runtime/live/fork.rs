use yo_core::session_repository::StoredSessionContinuation;
use yo_tui::ForkPickerToken;

use super::{
    super::{
        frontend,
        session::{LiveSession, SessionStep, shutdown_live_session},
        startup::{self, StartupOutcome, StartupSnapshots},
    },
    saved_execution_options,
};
use crate::{command, interaction::diagnostic::AppError, state::storage};

pub(super) fn fork_session(
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
