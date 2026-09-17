use std::sync::atomic::{AtomicU64, Ordering};

use super::{super::SessionState, AgentWorker};
use crate::{ActivityKind, ActivityRequestRef, AgentEvent, RuntimeError, RuntimePoll};

impl AgentWorker {
    pub(super) fn poll(&mut self) -> Result<RuntimePoll, RuntimeError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let poll = self.runtime.poll_event()?;
        if let RuntimePoll::Event(event) = &poll {
            apply_event(&mut state, &self.active_turn_id, event);
        }
        Ok(poll)
    }
}

pub(in crate::agent_session) fn apply_events(
    state: &mut SessionState,
    active_turn_id: &AtomicU64,
    events: &[AgentEvent],
) {
    for event in events {
        apply_event(state, active_turn_id, event);
    }
}

pub(in crate::agent_session) fn apply_event(
    state: &mut SessionState,
    active_turn_id: &AtomicU64,
    event: &AgentEvent,
) {
    match event {
        AgentEvent::ActivityStarted {
            activity,
            kind:
                ActivityKind::ApprovalRequest { request_id }
                | ActivityKind::UserInputRequest { request_id },
        } => {
            state
                .outstanding_requests
                .insert(ActivityRequestRef::new(*activity, *request_id));
        },
        AgentEvent::ActivityFinished { activity, .. } => {
            state
                .outstanding_requests
                .retain(|request| request.activity() != *activity);
        },
        AgentEvent::TurnStarted { turn } => {
            active_turn_id.store(turn.turn_id().get().get(), Ordering::Release);
            state.active_turn = Some(*turn);
            state.turn_started = true;
        },
        AgentEvent::TurnFinished { turn, .. } if state.active_turn == Some(*turn) => {
            active_turn_id.store(0, Ordering::Release);
            state.active_turn = None;
            state.turn_started = false;
            state.interrupted_turns.remove(turn);
        },
        _ => {},
    }
}
