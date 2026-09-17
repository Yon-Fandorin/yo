use std::path;

use super::{
    super::{
        frontend,
        session::{LiveSession, SessionStep},
        startup::{self, StartupFrontend, StartupOutcome, StartupSnapshots},
    },
    fork, new_session, presentation, resume,
};
use crate::{
    application::live_selection as selection, command, interaction::diagnostic::AppError,
    state::storage,
};

pub(super) fn run_generation(
    termination: &mut impl yo_tui::TerminationSource,
    live: &mut Option<LiveSession>,
    cwd: &path::Path,
    options: command::LiveOptions,
    launch_failure_selection: selection::LiveSelection,
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
        SessionStep::New => {
            new_session::start_new_session(termination, live, options, snapshots, None)
        },
        SessionStep::Interview(intent) => {
            new_session::start_new_session(termination, live, options, snapshots, Some(intent))
        },
        SessionStep::Fork => fork::fork_session(termination, live, options, snapshots, None),
        SessionStep::ForkPicker => {
            let current = live
                .as_mut()
                .expect("fork picker requires an existing Session");
            if let Err(error) = presentation::show_fork_picker(current) {
                current.tui.report_fork_failure(error.to_string());
            }
            Ok(SessionStep::Continue)
        },
        SessionStep::ForkBoundary { picker, index } => {
            fork::fork_session(termination, live, options, snapshots, Some((picker, index)))
        },
        SessionStep::Tree => {
            let current = live
                .as_mut()
                .expect("Session tree requires an existing Session");
            if let Err(error) = presentation::show_session_tree(current) {
                current.tui.report_session_tree_failure(error.to_string());
            }
            Ok(SessionStep::Continue)
        },
        SessionStep::Resume(target) => {
            resume::resume_session(termination, live, target, options, snapshots)
        },
        other => Ok(other),
    }
}
