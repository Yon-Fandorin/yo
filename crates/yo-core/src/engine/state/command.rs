use std::collections::HashMap;

use super::{EngineState, SessionState, TurnState};
use crate::{
    ActivityOutcome, ActivityRequestRef, ActivityResponse, AgentCommand, AgentEvent, SessionId,
    TurnOutcome, TurnRef, UserInput,
    engine::{AgentRejection, ResponseKind},
};

impl EngineState {
    pub(in crate::engine) fn handle_command(
        &mut self,
        command: AgentCommand,
    ) -> Result<Vec<AgentEvent>, AgentRejection> {
        match command {
            AgentCommand::CreateSession { session_id } => self.create_session(session_id),
            AgentCommand::StartTurn { turn, input } => self.start_turn(turn, input),
            AgentCommand::SteerTurn { turn, .. } => {
                self.require_active_turn_mut(turn)?;
                Err(AgentRejection::UnsupportedSteer)
            },
            AgentCommand::RespondToActivity { request, response } => {
                self.respond_to_activity(request, response)?;
                Ok(Vec::new())
            },
            AgentCommand::InterruptTurn { turn } => self.interrupt_turn(turn),
        }
    }

    fn create_session(&mut self, session_id: SessionId) -> Result<Vec<AgentEvent>, AgentRejection> {
        if let Some(session) = &self.session {
            return Err(AgentRejection::SessionAlreadyExists {
                existing: session.session_id,
                requested: session_id,
            });
        }
        self.session = Some(SessionState {
            session_id,
            turns: Vec::new(),
        });
        Ok(vec![AgentEvent::SessionCreated { session_id }])
    }

    fn start_turn(
        &mut self,
        turn: TurnRef,
        input: UserInput,
    ) -> Result<Vec<AgentEvent>, AgentRejection> {
        let session = self.require_session_mut(turn.session_id())?;
        if let Some(active) = session.active_turn() {
            return Err(AgentRejection::TurnAlreadyActive {
                active: active.turn,
            });
        }
        if session.turns.iter().any(|candidate| candidate.turn == turn) {
            return Err(AgentRejection::DuplicateTurn { turn });
        }
        session.turns.push(TurnState {
            turn,
            input,
            outcome: None,
            activities: HashMap::new(),
            activity_order: Vec::new(),
            requests: HashMap::new(),
            request_order: Vec::new(),
        });
        Ok(vec![AgentEvent::TurnStarted { turn }])
    }

    fn respond_to_activity(
        &mut self,
        request: ActivityRequestRef,
        response: ActivityResponse,
    ) -> Result<(), AgentRejection> {
        let turn = self.require_active_turn_mut(request.activity().turn())?;
        let request_state = turn
            .requests
            .get_mut(&request)
            .ok_or(AgentRejection::RequestNotFound { request })?;
        if request_state.answered {
            return Err(AgentRejection::RequestAlreadyAnswered { request });
        }

        let actual = response_kind(&response);
        if request_state.expected != actual.into() {
            return Err(AgentRejection::ResponseKindMismatch {
                request,
                expected: request_state.expected,
                actual,
            });
        }
        request_state.answered = true;
        Ok(())
    }

    fn interrupt_turn(&mut self, turn: TurnRef) -> Result<Vec<AgentEvent>, AgentRejection> {
        let turn_state = self.require_active_turn_mut(turn)?;
        let mut events = Vec::new();
        for activity in &turn_state.activity_order {
            let state = turn_state
                .activities
                .get_mut(activity)
                .expect("activity order and storage stay synchronized");
            if state.active {
                state.active = false;
                events.push(AgentEvent::ActivityFinished {
                    activity: *activity,
                    outcome: ActivityOutcome::Interrupted,
                });
            }
        }
        turn_state.outcome = Some(TurnOutcome::Interrupted);
        events.push(AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Interrupted,
        });
        Ok(events)
    }
}

fn response_kind(response: &ActivityResponse) -> ResponseKind {
    match response {
        ActivityResponse::Approval(_) => ResponseKind::Approval,
        ActivityResponse::UserInput(_) => ResponseKind::UserInput,
    }
}
