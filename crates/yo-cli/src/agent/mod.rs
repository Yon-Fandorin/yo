use std::collections::VecDeque;

use yo_core::{
    AgentBackend, AgentIntent, AgentSession, AgentSessionError, AgentSessionPoll, CommandAdmission,
    JournalSequence, PendingCommand, TranscriptReader, TranscriptRecord,
};
use yo_tui::{AgentConnection, AgentPoll, TerminationEvent, TerminationSource};

/// Adapts the frontend-independent Session to the TUI's connection boundary.
pub(crate) struct TuiAgentConnection {
    session: AgentSession,
    transcript: TranscriptReader,
    cursor: Option<JournalSequence>,
    pending: VecDeque<TranscriptRecord>,
    journal_changed: bool,
    closed: bool,
    failure: Option<AgentSessionError>,
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
        .map(|session| {
            session.map(|session| {
                let transcript = session.transcript_reader();
                Self {
                    session,
                    transcript,
                    cursor: None,
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

        if self.pending.is_empty() && self.journal_changed {
            let slice = self.transcript.read_after(self.cursor);
            let head = slice.head();
            for entry in slice.into_entries() {
                self.cursor = Some(entry.sequence());
                self.pending.push_back(entry.record().clone());
            }
            self.journal_changed = self.cursor != head;
        }

        if let Some(record) = self.pending.pop_front() {
            return Ok(AgentPoll::Record(record));
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
