mod activity;
mod command;

use std::collections::HashMap;

use super::{AgentRejection, ExpectedResponse};
use crate::{ActivityRef, ActivityRequestRef, SessionId, TurnOutcome, TurnRef, UserInput};

#[derive(Debug, Default)]
pub(super) struct EngineState {
    session: Option<SessionState>,
}

#[derive(Debug)]
struct SessionState {
    session_id: SessionId,
    turns: Vec<TurnState>,
}

#[derive(Debug)]
struct TurnState {
    turn: TurnRef,
    input: UserInput,
    outcome: Option<TurnOutcome>,
    interrupt_requested: bool,
    activities: HashMap<ActivityRef, ActivityState>,
    activity_order: Vec<ActivityRef>,
    requests: HashMap<ActivityRequestRef, RequestState>,
    request_order: Vec<ActivityRequestRef>,
}

#[derive(Debug)]
struct ActivityState {
    active: bool,
}

#[derive(Debug)]
struct RequestState {
    expected: ExpectedResponse,
    answered: bool,
    response_recorded: bool,
}

impl EngineState {
    pub(super) fn session_id(&self) -> Option<SessionId> {
        self.session.as_ref().map(|session| session.session_id)
    }

    pub(super) fn active_turn(&self) -> Option<TurnRef> {
        self.session
            .as_ref()
            .and_then(SessionState::active_turn)
            .map(|turn| turn.turn)
    }

    pub(super) fn active_turn_input(&self) -> Option<&UserInput> {
        self.session
            .as_ref()
            .and_then(SessionState::active_turn)
            .map(|turn| &turn.input)
    }

    pub(super) fn turn_count(&self) -> usize {
        self.session
            .as_ref()
            .map_or(0, |session| session.turns.len())
    }

    fn require_session_mut(
        &mut self,
        actual: SessionId,
    ) -> Result<&mut SessionState, AgentRejection> {
        let session = self
            .session
            .as_mut()
            .ok_or(AgentRejection::SessionNotCreated)?;
        if session.session_id != actual {
            return Err(AgentRejection::SessionMismatch {
                expected: session.session_id,
                actual,
            });
        }
        Ok(session)
    }

    fn require_session(&self, actual: SessionId) -> Result<&SessionState, AgentRejection> {
        let session = self
            .session
            .as_ref()
            .ok_or(AgentRejection::SessionNotCreated)?;
        if session.session_id != actual {
            return Err(AgentRejection::SessionMismatch {
                expected: session.session_id,
                actual,
            });
        }
        Ok(session)
    }

    fn require_active_turn(&self, turn: TurnRef) -> Result<&TurnState, AgentRejection> {
        let session = self.require_session(turn.session_id())?;
        session
            .active_turn()
            .filter(|active| active.turn == turn)
            .ok_or(AgentRejection::TurnNotActive { turn })
    }

    fn require_active_turn_mut(&mut self, turn: TurnRef) -> Result<&mut TurnState, AgentRejection> {
        let session = self.require_session_mut(turn.session_id())?;
        session
            .active_turn_mut()
            .filter(|active| active.turn == turn)
            .ok_or(AgentRejection::TurnNotActive { turn })
    }

    fn require_active_activity_mut(
        &mut self,
        activity: ActivityRef,
    ) -> Result<&mut ActivityState, AgentRejection> {
        let turn = self.require_active_turn_mut(activity.turn())?;
        turn.activities
            .get_mut(&activity)
            .ok_or(AgentRejection::ActivityNotActive { activity })
    }
}

impl SessionState {
    fn active_turn(&self) -> Option<&TurnState> {
        self.turns.iter().rev().find(|turn| turn.outcome.is_none())
    }

    fn active_turn_mut(&mut self) -> Option<&mut TurnState> {
        self.turns
            .iter_mut()
            .rev()
            .find(|turn| turn.outcome.is_none())
    }
}
