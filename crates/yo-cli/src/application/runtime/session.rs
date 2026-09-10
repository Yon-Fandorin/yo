use std::task::{Context, Poll, Waker};

use yo_core::{AgentSessionError, session_repository::StoredSessionForkCatalog};
use yo_tui::{AgentPoll, ForkPickerToken, TerminationEvent, TerminationSource};

use crate::{
    application::agent,
    execution::{model, tools as local_tools},
    interaction::diagnostic::AppError,
};

pub(super) struct LiveSession {
    pub(super) session_id: yo_core::SessionId,
    pub(super) presentation: super::presentation::Presentation,
    pub(super) agent: agent::TuiAgentConnection,
    pub(super) pending_diagnostic_poll: Option<Result<AgentPoll, AgentSessionError>>,
    pub(super) tui: yo_tui::TuiSession,
    pub(super) workspace: std::path::PathBuf,
    pub(super) local_tool_registry: Option<local_tools::LocalToolRegistryRevision>,
    pub(super) execution_manifest_digest: Option<String>,
    pub(super) fork_catalog: Option<(ForkPickerToken, StoredSessionForkCatalog)>,
    pub(super) active_host: Option<yo_core::HostId>,
    pub(super) active_host_execution: Option<model::DelegatedExecutionProfile>,
    pub(super) active_host_model: Option<model::ActiveHostModel>,
    pub(super) startup_target: yo_core::StartupTarget,
    pub(super) host_catalogs: Vec<model::HostCatalogObservation>,
}

pub(super) enum SessionStep {
    Suspend,
    Continue,
    New,
    Fork,
    ForkPicker,
    ForkBoundary {
        picker: ForkPickerToken,
        index: usize,
    },
    Tree,
    Resume(Option<yo_core::SessionId>),
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

pub(super) fn termination_requested(termination: &mut impl TerminationSource) -> bool {
    let mut context = Context::from_waker(Waker::noop());
    termination.poll_termination(&mut context) == Poll::Ready(TerminationEvent::Requested)
}
