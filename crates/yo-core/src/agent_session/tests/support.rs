use std::{
    num::NonZeroU64,
    thread,
    time::{Duration, Instant},
};

use super::super::{AgentSession, AgentSessionError};
use crate::{
    ActivityId, ActivityRef, AgentBackend, AgentEvent, RuntimePoll, SessionId, TurnId, TurnRef,
};

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

pub(super) fn start_app(backend: impl AgentBackend + Send + 'static) -> AgentSession {
    let mut app = AgentSession::start(backend).unwrap();
    assert_eq!(
        next_poll(&mut app).unwrap(),
        RuntimePoll::Event(AgentEvent::SessionCreated {
            session_id: session(),
        })
    );
    app
}

pub(super) fn next_poll(app: &mut AgentSession) -> Result<RuntimePoll, AgentSessionError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match app.poll()? {
            RuntimePoll::Pending if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(1));
            },
            RuntimePoll::Pending => {
                return Err(AgentSessionError::WorkerUnavailable(
                    "test timed out waiting for a worker observation".to_owned(),
                ));
            },
            observation => return Ok(observation),
        }
    }
}
