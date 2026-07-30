use std::{
    collections::VecDeque,
    num::NonZeroU64,
    ops::{Deref, DerefMut},
    thread,
    time::{Duration, Instant},
};

use super::super::{AgentSession, AgentSessionError, AgentSessionPoll};
use crate::{
    ActivityId, ActivityRef, AgentBackend, JournalSequence, RuntimePoll, SessionId,
    TranscriptReader, TranscriptRecord, TurnId, TurnRef,
};

pub(super) struct TestApp {
    session: AgentSession,
    transcript: TranscriptReader,
    cursor: Option<JournalSequence>,
    observed_head: Option<JournalSequence>,
    pending: VecDeque<TranscriptRecord>,
}

impl Deref for TestApp {
    type Target = AgentSession;

    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.session
    }
}

pub(super) fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

pub(super) fn session() -> SessionId {
    SessionId::new(id(1))
}

pub(super) fn turn(value: u64) -> TurnRef {
    TurnRef::new(session(), TurnId::new(id(value)))
}

pub(super) fn activity(turn: TurnRef, value: u64) -> ActivityRef {
    ActivityRef::new(turn, ActivityId::new(id(value)))
}

pub(super) fn start_app(backend: impl AgentBackend + Send + 'static) -> TestApp {
    let agent_session = AgentSession::start(backend).unwrap();
    let transcript = agent_session.transcript_reader();
    let mut app = TestApp {
        session: agent_session,
        transcript,
        cursor: None,
        observed_head: None,
        pending: VecDeque::new(),
    };
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(crate::AgentEvent::SessionCreated {
            session_id: session(),
        })
    );
    app
}

pub(super) fn next_poll(app: &mut TestApp) -> Result<RuntimePoll, AgentSessionError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        while let Some(record) = app.pending.pop_front() {
            if let TranscriptRecord::EventCommitted(event) = record {
                return Ok(RuntimePoll::Event(event));
            }
        }
        if app.cursor != app.observed_head {
            let slice = app.transcript.read_after(app.cursor);
            if let Some(last) = slice.entries().last() {
                app.cursor = Some(last.sequence());
            }
            app.pending.extend(
                slice
                    .into_entries()
                    .into_iter()
                    .map(|entry| entry.record().clone()),
            );
            continue;
        }

        match app.session.poll()? {
            AgentSessionPoll::Changed => {
                app.observed_head = app.transcript.head_sequence();
            },
            AgentSessionPoll::Pending if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(1));
            },
            AgentSessionPoll::Pending => {
                return Err(AgentSessionError::WorkerUnavailable(
                    "test timed out waiting for a worker observation".to_owned(),
                ));
            },
            AgentSessionPoll::Closed => return Ok(RuntimePoll::Closed),
        }
    }
}
