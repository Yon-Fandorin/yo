use std::collections::HashMap;

use super::{EngineState, SessionState, TurnState};
use crate::{
    ActivityRequestRef, ActivityResponse, AgentCommand, AgentEvent, SessionId, TurnRef, UserInput,
    engine::{AgentRejection, ResponseKind},
};

impl EngineState {
    pub(in crate::engine) fn validate_command(
        &self,
        command: &AgentCommand,
        supports_steer: bool,
    ) -> Result<(), AgentRejection> {
        match command {
            AgentCommand::CreateSession { session_id } => {
                if let Some(session) = &self.session {
                    return Err(AgentRejection::SessionAlreadyExists {
                        existing: session.session_id,
                        requested: *session_id,
                    });
                }
                Ok(())
            },
            AgentCommand::StartTurn { turn, .. } => {
                let session = self.require_session(turn.session_id())?;
                if let Some(active) = session.active_turn() {
                    return Err(AgentRejection::TurnAlreadyActive {
                        active: active.turn,
                    });
                }
                if session
                    .turns
                    .iter()
                    .any(|candidate| candidate.turn == *turn)
                {
                    return Err(AgentRejection::DuplicateTurn { turn: *turn });
                }
                Ok(())
            },
            AgentCommand::SteerTurn { turn, .. } => {
                self.require_active_turn(*turn)?;
                if supports_steer {
                    Ok(())
                } else {
                    Err(AgentRejection::UnsupportedSteer)
                }
            },
            AgentCommand::RespondToActivity { request, response } => {
                self.validate_activity_response(*request, response)
            },
            AgentCommand::InterruptTurn { turn } => {
                let turn_state = self.require_active_turn(*turn)?;
                if turn_state.interrupt_requested {
                    Err(AgentRejection::InterruptAlreadyRequested { turn: *turn })
                } else {
                    Ok(())
                }
            },
            AgentCommand::CompactContext { guidance } => {
                let Some(session) = self.session.as_ref() else {
                    return Err(AgentRejection::SessionNotCreated);
                };
                if let Some(active) = session.active_turn() {
                    return Err(AgentRejection::TurnAlreadyActive {
                        active: active.turn,
                    });
                }
                if guidance.as_ref().is_some_and(|guidance| {
                    guidance.is_empty()
                        || guidance.trim() != guidance
                        || guidance.len() > 8 * 1024
                        || guidance.chars().any(|character| {
                            character.is_control() && !matches!(character, '\n' | '\t')
                        })
                }) {
                    return Err(AgentRejection::InvalidContextCompactionGuidance);
                }
                Ok(())
            },
        }
    }

    pub(in crate::engine) fn handle_command(
        &mut self,
        command: AgentCommand,
        supports_steer: bool,
    ) -> Result<Vec<AgentEvent>, AgentRejection> {
        self.validate_command(&command, supports_steer)?;
        match command {
            AgentCommand::CreateSession { session_id } => self.create_session(session_id),
            AgentCommand::StartTurn { turn, input } => self.start_turn(turn, input),
            AgentCommand::SteerTurn { .. } => Ok(Vec::new()),
            AgentCommand::RespondToActivity { request, response } => {
                self.respond_to_activity(request, response)?;
                Ok(Vec::new())
            },
            AgentCommand::InterruptTurn { turn } => self.interrupt_turn(turn),
            AgentCommand::CompactContext { .. } => Ok(Vec::new()),
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
            interrupt_requested: false,
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

    fn validate_activity_response(
        &self,
        request: ActivityRequestRef,
        response: &ActivityResponse,
    ) -> Result<(), AgentRejection> {
        let turn = self.require_active_turn(request.activity().turn())?;
        let request_state = turn
            .requests
            .get(&request)
            .ok_or(AgentRejection::RequestNotFound { request })?;
        if request_state.answered {
            return Err(AgentRejection::RequestAlreadyAnswered { request });
        }

        let actual = response_kind(response);
        if request_state.expected != actual.into() {
            return Err(AgentRejection::ResponseKindMismatch {
                request,
                expected: request_state.expected,
                actual,
            });
        }
        Ok(())
    }

    fn interrupt_turn(&mut self, turn: TurnRef) -> Result<Vec<AgentEvent>, AgentRejection> {
        let turn_state = self.require_active_turn_mut(turn)?;
        turn_state.interrupt_requested = true;
        Ok(Vec::new())
    }
}

fn response_kind(response: &ActivityResponse) -> ResponseKind {
    match response {
        ActivityResponse::Approval(_) => ResponseKind::Approval,
        ActivityResponse::UserInput(_) => ResponseKind::UserInput,
    }
}
