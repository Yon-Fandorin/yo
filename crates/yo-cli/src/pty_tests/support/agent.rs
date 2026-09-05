use std::{
    collections::VecDeque,
    io,
    num::NonZeroU64,
    task::{Context, Poll},
};

use yo_core::{AgentCommand, TranscriptRecord, TurnId, TurnRef, UserInput};
use yo_tui::{AgentAction, AgentConnection, AgentPoll, DispatchOutcome, PendingDispatch};

pub(in crate::pty_tests) struct PendingAgent;

impl AgentConnection for PendingAgent {
    type Error = io::Error;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(AgentPoll::Pending)
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

pub(in crate::pty_tests) struct RetainedChatAgent {
    records: VecDeque<TranscriptRecord>,
}

impl RetainedChatAgent {
    pub(in crate::pty_tests) fn new() -> Self {
        Self::with_input("YO_INLINE_RETAINED")
    }

    pub(in crate::pty_tests) fn new_with_large_publication() -> Self {
        Self::with_input(format!("YO_INLINE_RETAINED{}", " x".repeat(48 * 1024)))
    }

    fn with_input(input: impl Into<UserInput>) -> Self {
        let session_id = "01890f00-0000-7000-8000-000000000001"
            .parse()
            .expect("the fixture is a UUIDv7");
        let turn = TurnRef::new(session_id, id(TurnId::new));
        Self {
            records: [TranscriptRecord::CommandCommitted(
                AgentCommand::StartTurn {
                    turn,
                    input: input.into(),
                },
            )]
            .into(),
        }
    }
}

impl AgentConnection for RetainedChatAgent {
    type Error = io::Error;

    fn dispatch(&mut self, _action: AgentAction) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn retry(&mut self, _pending: PendingDispatch) -> Result<DispatchOutcome, Self::Error> {
        Ok(DispatchOutcome::Queued)
    }

    fn poll(&mut self) -> Result<AgentPoll, Self::Error> {
        Ok(self
            .records
            .pop_front()
            .map_or(AgentPoll::Pending, AgentPoll::Record))
    }

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<()> {
        if self.records.is_empty() {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

fn id<T>(constructor: impl FnOnce(NonZeroU64) -> T) -> T {
    constructor(NonZeroU64::MIN)
}
