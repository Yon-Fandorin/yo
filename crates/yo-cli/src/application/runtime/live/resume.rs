use super::{
    super::{
        frontend,
        session::{LiveSession, SessionStep, shutdown_live_session},
        startup::{self, StartupFrontend, StartupOutcome, StartupSnapshots},
    },
    presentation::show_resume_picker,
    saved_execution_options,
};
use crate::{application::live_selection as selection, command, interaction::diagnostic::AppError};

pub(super) fn resume_session(
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
    let preparation =
        selection::prepare(selection::LiveSelection::Resume(target), &current.workspace);
    match preparation {
        Ok(selection::LivePreparation::Resume { .. }) => {},
        Ok(selection::LivePreparation::ReadOnly { reason, .. }) => {
            current.tui.report_resume_failure(format!(
                "{reason}. Inspect saved history with `yo session {target}`."
            ));
            return Ok(SessionStep::Continue);
        },
        Ok(selection::LivePreparation::New) => {
            unreachable!("explicit resume cannot start a new Session")
        },
        Err(error) => {
            current.tui.report_resume_failure(error.to_string());
            return Ok(SessionStep::Continue);
        },
    }
    // Resume은 저장된 binding 하나로 선택하며 현재 Session의 override가 섞이지 않게 한다.
    let options = saved_execution_options(options, command::LiveSelection::Resume(target));
    let mut resumed_snapshots = StartupSnapshots {
        config: snapshots.config,
        credentials: snapshots.credentials,
        stored_preference: None,
        codex_warnings: snapshots.codex_warnings,
    };
    // Continue의 failure disposition은 archival stdout을 쓰고 terminal flow를 닫는 대신 중단한다.
    // 현재 live Session은 열린 채 정확한 error를 받는다.
    let prepared = startup::prepare_agent(
        termination,
        &current.workspace,
        &options,
        selection::LiveSelection::Continue,
        None,
        &mut resumed_snapshots,
        StartupFrontend::Terminal,
    );
    let candidate = match prepared {
        Ok(StartupOutcome::Ready(prepared)) => {
            match frontend::build_live_session(
                *prepared,
                snapshots.config,
                &options,
                snapshots.credentials.as_ref(),
            ) {
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
