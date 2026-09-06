use std::{
    collections::VecDeque,
    task::{Context, Poll},
};

use yo_core::{
    AgentIntent, AgentSession, AgentSessionError, AgentSessionPoll, CommandAdmission,
    PendingCommand,
};
use yo_tui::{AgentConnection, AgentPoll};

use super::journal::JournalState;

/// Adapts the frontend-independent Session to the TUI's connection boundary.
pub(crate) struct TuiAgentConnection {
    session: AgentSession,
    journal: JournalState,
    pending: VecDeque<AgentPoll>,
    closed: bool,
    failure: Option<AgentSessionError>,
}

impl TuiAgentConnection {
    pub(in crate::agent) fn from_session(session: AgentSession) -> Self {
        Self {
            journal: JournalState::from_session(&session),
            session,
            pending: VecDeque::new(),
            closed: false,
            failure: None,
        }
    }

    pub(crate) fn into_session(self) -> AgentSession {
        self.session
    }

    /// Returns the initial durable binding fact after the startup handshake has completed.
    pub(crate) fn initial_binding_record(&self) -> Option<yo_core::RequestTraceRecord> {
        self.journal.initial_binding_record()
    }

    #[cfg(test)]
    pub(in crate::agent) fn transcript_head_sequence(&self) -> Option<yo_core::JournalSequence> {
        self.journal.transcript_head_sequence()
    }

    pub(in crate::agent) fn shutdown_session(
        &mut self,
    ) -> Result<Vec<yo_core::AgentEvent>, AgentSessionError> {
        self.session.shutdown()
    }

    pub(in crate::agent) fn replace_session_backend(
        &mut self,
        backend: Box<dyn yo_core::AgentBackend + Send>,
        termination_source: &mut impl yo_tui::TerminationSource,
    ) -> Result<yo_core::BackendReplacementOutcome, AgentSessionError> {
        self.session.replace_backend(backend, || {
            super::termination::requested(termination_source)
        })
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

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        match self.session.poll() {
            Ok(AgentSessionPoll::Pending) => {},
            Ok(AgentSessionPoll::Changed) => self.journal.mark_changed(),
            Ok(AgentSessionPoll::Closed) => {
                self.journal.mark_changed();
                self.closed = true;
            },
            Err(error) => {
                self.journal.mark_changed();
                self.failure = Some(error);
                self.closed = true;
            },
        }

        while let Some(outcome) = self.session.take_submission_outcome() {
            self.pending.push_back(AgentPoll::Submission(outcome));
        }
        while let Some(outcome) = self.session.take_control_outcome() {
            self.pending.push_back(AgentPoll::Control(outcome));
        }

        if self.pending.is_empty() {
            self.journal.drain_into(&mut self.pending);
        }

        if let Some(observation) = self.pending.pop_front() {
            return Ok(observation);
        }
        if let Some(error) = self.failure.take() {
            return Err(error);
        }
        if self.closed {
            return Ok(AgentPoll::Closed);
        }
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<()> {
        if !self.pending.is_empty()
            || self.journal.is_changed()
            || self.failure.is_some()
            || self.closed
        {
            Poll::Ready(())
        } else {
            self.session.poll_ready(context)
        }
    }
}
