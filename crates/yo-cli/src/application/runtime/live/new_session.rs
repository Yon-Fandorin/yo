use super::super::{
    frontend,
    session::{LiveSession, SessionStep, shutdown_live_session},
    startup::{self, StartupOutcome, StartupSnapshots},
};
use crate::{
    command, execution::tools::LocalToolRegistryRevision, interaction::diagnostic::AppError,
};

pub(super) fn start_new_session(
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
            match frontend::build_live_session(
                *prepared,
                snapshots.config,
                &options,
                snapshots.credentials.as_ref(),
            ) {
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
    // 별도의 writer/backend를 먼저 준비하므로 위 단계가 실패해도 기존 Session을 유지한다.
    let mut previous = live.replace(candidate);
    if let Err(error) = shutdown_live_session(&mut previous) {
        // 새 Session을 이미 선택했으므로 cleanup failure를 계속 표시해야 한다.
        live.as_mut()
            .expect("installed new Session")
            .tui
            .report_new_session_cleanup_failure(error.to_string());
    }
    Ok(SessionStep::Continue)
}
