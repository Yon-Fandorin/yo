use crate::{agent, diagnostic::AppError, local_tools, model};

pub(super) struct LiveSession {
    pub(super) agent: agent::TuiAgentConnection,
    pub(super) tui: yo_tui::TuiSession,
    pub(super) workspace: std::path::PathBuf,
    pub(super) local_tool_registry: Option<local_tools::LocalToolRegistryRevision>,
    pub(super) active_host: Option<yo_core::HostId>,
    pub(super) active_host_execution: Option<model::DelegatedExecutionProfile>,
    pub(super) active_host_model: Option<model::ActiveHostModel>,
    pub(super) host_catalogs: Vec<model::HostCatalogObservation>,
}

pub(super) enum SessionStep {
    Suspend,
    Continue,
    Complete,
}

pub(super) fn shutdown_live_session(live: &mut Option<LiveSession>) -> Result<(), AppError> {
    let Some(mut session) = live.take() else {
        return Ok(());
    };
    session
        .agent
        .shutdown()
        .map(drop)
        .map_err(|error| AppError::single("agent cleanup", error))
}
