use yo_core::{AgentBackend, AgentSession, AgentSessionError, SessionDescriptor};
use yo_tui::TerminationSource;

use super::{TuiAgentConnection, termination};

impl TuiAgentConnection {
    #[cfg(test)]
    pub(crate) fn start<B>(
        backend: B,
        session_id: yo_core::SessionId,
        termination_source: &mut impl TerminationSource,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        AgentSession::start_cancellable_with_id(backend, session_id, || {
            termination::requested(termination_source)
        })
        .map(|session| session.map(Self::from_session))
    }

    pub(crate) fn start_persistent<B, R>(
        backend: B,
        descriptor: SessionDescriptor,
        repository: R,
        termination_source: &mut impl TerminationSource,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
        R: yo_core::session_repository::SessionRepository + Send + 'static,
    {
        AgentSession::start_cancellable_with_repository(backend, descriptor, repository, || {
            termination::requested(termination_source)
        })
        .map(|session| session.map(Self::from_session))
    }

    pub(crate) fn start_resumed<B, R>(
        backend: B,
        continuation: yo_core::session_repository::StoredSessionContinuation,
        repository: R,
        replace_binding: bool,
        termination_source: &mut impl TerminationSource,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
        R: yo_core::session_repository::SessionRepository + Send + 'static,
    {
        let started = if replace_binding {
            AgentSession::start_cancellable_with_replacement_continuation(
                backend,
                continuation,
                repository,
                || termination::requested(termination_source),
            )
        } else {
            AgentSession::start_cancellable_with_continuation(
                backend,
                continuation,
                repository,
                || termination::requested(termination_source),
            )
        };
        started.map(|session| session.map(Self::from_session))
    }

    pub(crate) fn shutdown(&mut self) -> Result<Vec<yo_core::AgentEvent>, AgentSessionError> {
        self.shutdown_session()
    }

    pub(crate) fn replace_backend(
        &mut self,
        backend: Box<dyn AgentBackend + Send>,
        termination_source: &mut impl TerminationSource,
    ) -> Result<yo_core::BackendReplacementOutcome, AgentSessionError> {
        self.replace_session_backend(backend, termination_source)
    }
}
