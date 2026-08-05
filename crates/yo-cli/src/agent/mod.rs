use std::collections::VecDeque;

#[cfg(test)]
use yo_core::SessionId;
use yo_core::{
    AgentBackend, AgentIntent, AgentSession, AgentSessionError, AgentSessionPoll, CommandAdmission,
    JournalSequence, PendingCommand, RequestTraceReader, SessionDescriptor, TranscriptObservation,
    TranscriptObservationSequence, TranscriptReader, session_repository::SessionRepository,
};
use yo_tui::{AgentConnection, AgentPoll, TerminationEvent, TerminationSource};

/// Adapts the frontend-independent Session to the TUI's connection boundary.
pub(crate) struct TuiAgentConnection {
    session: AgentSession,
    transcript: TranscriptReader,
    request_trace: RequestTraceReader,
    cursor: Option<TranscriptObservationSequence>,
    request_cursor: Option<JournalSequence>,
    pending: VecDeque<AgentPoll>,
    journal_changed: bool,
    closed: bool,
    failure: Option<AgentSessionError>,
}

impl TuiAgentConnection {
    #[cfg(test)]
    pub(crate) fn start<B>(
        backend: B,
        session_id: SessionId,
        termination: &mut impl TerminationSource,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
    {
        AgentSession::start_cancellable_with_id(backend, session_id, || {
            termination.poll_termination() == TerminationEvent::Requested
        })
        .map(|session| {
            session.map(|session| {
                let transcript = session.transcript_reader();
                let request_trace = session.request_trace_reader();
                Self {
                    session,
                    transcript,
                    request_trace,
                    cursor: None,
                    request_cursor: None,
                    pending: VecDeque::new(),
                    journal_changed: false,
                    closed: false,
                    failure: None,
                }
            })
        })
    }

    pub(crate) fn start_persistent<B, R>(
        backend: B,
        descriptor: SessionDescriptor,
        repository: R,
        termination: &mut impl TerminationSource,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
        R: SessionRepository + Send + 'static,
    {
        AgentSession::start_cancellable_with_repository(backend, descriptor, repository, || {
            termination.poll_termination() == TerminationEvent::Requested
        })
        .map(|session| {
            session.map(|session| {
                let transcript = session.transcript_reader();
                let request_trace = session.request_trace_reader();
                Self {
                    session,
                    transcript,
                    request_trace,
                    cursor: None,
                    request_cursor: None,
                    pending: VecDeque::new(),
                    journal_changed: false,
                    closed: false,
                    failure: None,
                }
            })
        })
    }

    pub(crate) fn start_resumed<B, R>(
        backend: B,
        continuation: yo_core::session_repository::StoredSessionContinuation,
        repository: R,
        termination: &mut impl TerminationSource,
    ) -> Result<Option<Self>, AgentSessionError>
    where
        B: AgentBackend + Send + 'static,
        R: SessionRepository + Send + 'static,
    {
        AgentSession::start_cancellable_with_continuation(backend, continuation, repository, || {
            termination.poll_termination() == TerminationEvent::Requested
        })
        .map(|session| {
            session.map(|session| {
                let transcript = session.transcript_reader();
                let request_trace = session.request_trace_reader();
                Self {
                    session,
                    transcript,
                    request_trace,
                    cursor: None,
                    request_cursor: None,
                    pending: VecDeque::new(),
                    journal_changed: false,
                    closed: false,
                    failure: None,
                }
            })
        })
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

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        match self.session.poll() {
            Ok(AgentSessionPoll::Pending) => {},
            Ok(AgentSessionPoll::Changed) => self.journal_changed = true,
            Ok(AgentSessionPoll::Closed) => {
                self.journal_changed = true;
                self.closed = true;
            },
            Err(error) => {
                self.journal_changed = true;
                self.failure = Some(error);
                self.closed = true;
            },
        }

        while let Some(outcome) = self.session.take_submission_outcome() {
            self.pending.push_back(AgentPoll::Submission(outcome));
        }

        if self.pending.is_empty() && self.journal_changed {
            let slice = self.transcript.read_observations_after(self.cursor);
            let head = slice.head();
            for entry in slice.into_entries() {
                self.cursor = Some(entry.sequence());
                self.pending.push_back(match entry.observation() {
                    TranscriptObservation::Durability(durability) => {
                        AgentPoll::Durability(*durability)
                    },
                    TranscriptObservation::Record(record) => AgentPoll::Record(record.clone()),
                });
            }
            let trace = self.request_trace.read_after(self.request_cursor);
            let trace_head = trace.head();
            for entry in trace.into_entries() {
                self.request_cursor = Some(entry.sequence());
                self.pending.push_back(AgentPoll::RequestTrace(entry));
            }
            self.journal_changed = self.cursor != head || self.request_cursor != trace_head;
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
}

#[cfg(test)]
mod tests;
