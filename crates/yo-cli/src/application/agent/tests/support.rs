use std::{
    num::NonZeroU64,
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

use yo_core::{AgentIntent, CommandAdmission, SessionId, TranscriptRecord, TurnId, TurnRef};
use yo_tui::{AgentConnection, AgentPoll, TerminationEvent, TerminationSource};

use crate::application::agent::TuiAgentConnection;

/// Finite guard for a test worker that failed to publish an already-selected typed boundary.
///
/// Tests still complete only after observing their exact record or signal. This duration is not
/// a semantic completion oracle; it only prevents a broken test from hanging indefinitely under
/// slow full-suite scheduling.
pub(super) const TEST_DEADLOCK_GUARD: Duration = Duration::from_secs(30);

const COMMITTED_RECORDS_TIMEOUT: &str = "timed out waiting for committed records";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaitingPoll {
    Pending,
    Submission,
    RequestTrace,
}

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
            CommandAdmission::Rejected { rejection, .. } => {
                return Err(format!(
                    "test command was rejected: {}",
                    rejection.message()
                ));
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
            Ok(AgentPoll::Pending) => wait_for_more(WaitingPoll::Pending, deadline)?,
            Ok(AgentPoll::Submission(_)) => wait_for_more(WaitingPoll::Submission, deadline)?,
            Ok(AgentPoll::RequestTrace(_)) => wait_for_more(WaitingPoll::RequestTrace, deadline)?,
            Ok(other) => {
                return Err(unexpected_poll_message(&other));
            },
            Err(error) => return Err(error.to_string()),
        }
        if Instant::now() >= deadline {
            return Err(COMMITTED_RECORDS_TIMEOUT.to_owned());
        }
    }
}

fn wait_for_more(waiting: WaitingPoll, deadline: Instant) -> Result<(), String> {
    wait_for_more_at(waiting, Instant::now(), deadline)
}

fn wait_for_more_at(
    waiting: WaitingPoll,
    observed_at: Instant,
    deadline: Instant,
) -> Result<(), String> {
    match (waiting, observed_at < deadline) {
        (WaitingPoll::Pending | WaitingPoll::Submission | WaitingPoll::RequestTrace, true) => {
            thread::yield_now();
            Ok(())
        },
        (WaitingPoll::Pending | WaitingPoll::Submission | WaitingPoll::RequestTrace, false) => {
            Err(COMMITTED_RECORDS_TIMEOUT.to_owned())
        },
    }
}

fn unexpected_poll_message(poll: &AgentPoll) -> String {
    format!("connection ended before the expected records: {poll:?}")
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    // 세 waiting variant가 deadline을 넘으면 연결 종료로 오분류되지 않고 같은
    // bounded timeout 진단을 반환하는지 검증합니다.
    #[test]
    fn every_waiting_poll_uses_the_committed_records_timeout_after_deadline() {
        let deadline = Instant::now();
        for waiting in [
            WaitingPoll::Pending,
            WaitingPoll::Submission,
            WaitingPoll::RequestTrace,
        ] {
            assert_eq!(
                wait_for_more_at(waiting, deadline, deadline),
                Err(COMMITTED_RECORDS_TIMEOUT.to_owned())
            );
        }
    }

    // 실제 Closed 관찰은 waiting timeout과 섞이지 않고 기존 premature-end 진단을
    // 유지하는지 검증합니다.
    #[test]
    fn closed_poll_keeps_the_premature_connection_end_diagnostic() {
        assert_eq!(
            unexpected_poll_message(&AgentPoll::Closed),
            "connection ended before the expected records: Closed"
        );
    }
}
