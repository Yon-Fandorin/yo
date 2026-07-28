use yo_core::{
    AgentBackend, AgentIntent, AgentSession, AgentSessionError, CommandAdmission, PendingCommand,
    RuntimePoll,
};
use yo_tui::{AgentConnection, TerminationEvent, TerminationSource};

/// Adapts the frontend-independent Session to the TUI's connection boundary.
pub(crate) struct TuiAgentConnection {
    session: AgentSession,
}

impl TuiAgentConnection {
    pub(crate) fn start<B>(
        backend: B,
        termination: &mut impl TerminationSource,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        AgentSession::start_cancellable(backend, || {
            termination.poll_termination() == TerminationEvent::Requested
        })
        .map(|session| session.map(|session| Self { session }))
    }

    pub(crate) fn shutdown(&mut self) -> Result<Vec<yo_core::AgentEvent>, AgentSessionError> {
        self.session.shutdown()
    }
}

impl AgentConnection for TuiAgentConnection {
    type Error = AgentSessionError;

    fn dispatch(&mut self, action: AgentIntent) -> Result<CommandAdmission, Self::Error> {
        self.session.dispatch(action)
    }

    fn retry(&mut self, pending: PendingCommand) -> Result<CommandAdmission, Self::Error> {
        self.session.retry(pending)
    }

    fn poll(&mut self) -> Result<RuntimePoll, Self::Error> {
        self.session.poll()
    }
}
