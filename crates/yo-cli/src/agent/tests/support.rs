use std::{
    num::NonZeroU64,
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

use yo_core::{AgentIntent, CommandAdmission, SessionId, TranscriptRecord, TurnId, TurnRef};
use yo_tui::{AgentConnection, AgentPoll, TerminationEvent, TerminationSource};

use crate::agent::TuiAgentConnection;

/// Finite guard for a test worker that failed to publish an already-selected typed boundary.
///
/// Tests still complete only after observing their exact record or signal. This duration is not
/// a semantic completion oracle; it only prevents a broken test from hanging indefinitely under
/// slow full-suite scheduling.
pub(super) const TEST_DEADLOCK_GUARD: Duration = Duration::from_secs(30);

pub(super) struct NeverTerminated;

impl TerminationSource for NeverTerminated {
    fn poll_termination(&mut self, _context: &mut Context<'_>) -> Poll<TerminationEvent> {
        Poll::Pending
    }
}

pub(super) fn session_id() -> SessionId {
    "01890f00-0000-7000-8000-000000000001"
        .parse()
        .expect("the fixture is a UUIDv7")
}

pub(super) fn turn() -> TurnRef {
    TurnRef::new(session_id(), TurnId::new(NonZeroU64::MIN))
}

pub(super) fn dispatch_until_queued(
    connection: &mut TuiAgentConnection,
    intent: AgentIntent,
) -> Result<(), String> {
    let deadline = Instant::now() + TEST_DEADLOCK_GUARD;
    let mut admission = connection
        .dispatch(intent)
        .map_err(|error| error.to_string())?;
    loop {
        match admission {
            CommandAdmission::Queued => return Ok(()),
            CommandAdmission::Backpressured(pending) if Instant::now() < deadline => {
                thread::yield_now();
                admission = connection
                    .retry(pending)
                    .map_err(|error| error.to_string())?;
            },
            CommandAdmission::Backpressured(_) => {
                return Err("timed out retrying a backpressured test command".to_owned());
            },
        }
    }
}

pub(super) fn collect_until(
    connection: &mut TuiAgentConnection,
    done: impl Fn(&[TranscriptRecord]) -> bool,
) -> Result<Vec<TranscriptRecord>, String> {
    let deadline = Instant::now() + TEST_DEADLOCK_GUARD;
    let mut records = Vec::new();
    loop {
        match connection.poll() {
            Ok(AgentPoll::Record(record)) => {
                records.push(record);
                if done(&records) {
                    return Ok(records);
                }
            },
            Ok(AgentPoll::Pending | AgentPoll::Submission(_) | AgentPoll::RequestTrace(_))
                if Instant::now() < deadline =>
            {
                thread::yield_now();
            },
            Ok(other) => {
                return Err(format!(
                    "connection ended before the expected records: {other:?}"
                ));
            },
            Err(error) => return Err(error.to_string()),
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for committed records".to_owned());
        }
    }
}
