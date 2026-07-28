use std::{
    collections::HashSet,
    num::NonZeroU64,
    sync::{Arc, TryLockError, atomic::Ordering, mpsc::TrySendError},
};

use super::{AgentIntent, AgentSession, AgentSessionError, CommandAdmission, PendingCommand};
use crate::{ActivityRequestRef, ActivityResponse, AgentCommand, TurnId, TurnRef, UserInput};

#[derive(Default)]
pub(super) struct SessionState {
    pub(super) active_turn: Option<TurnRef>,
    pub(super) turn_started: bool,
    pub(super) interrupted_turns: HashSet<TurnRef>,
    pub(super) outstanding_requests: HashSet<ActivityRequestRef>,
}

impl AgentSession {
    fn resolve_action(
        &mut self,
        action: AgentIntent,
        state: &mut SessionState,
    ) -> Result<AgentCommand, AgentSessionError> {
        match action {
            AgentIntent::Submit(text) => {
                let input = UserInput::new(text);
                if let Some(turn) = state.active_turn {
                    if state.interrupted_turns.contains(&turn) {
                        return Err(AgentSessionError::TurnInterruptPending);
                    }
                    Ok(AgentCommand::SteerTurn { turn, input })
                } else {
                    let turn = self.next_turn()?;
                    state.active_turn = Some(turn);
                    state.turn_started = false;
                    self.active_turn_id
                        .store(turn.turn_id().get().get(), Ordering::Release);
                    Ok(AgentCommand::StartTurn { turn, input })
                }
            },
            AgentIntent::Interrupt => Ok(AgentCommand::InterruptTurn {
                turn: state.active_turn.ok_or(AgentSessionError::NoActiveTurn)?,
            }),
            AgentIntent::RespondToApproval { request, decision } => {
                if !state.outstanding_requests.contains(&request) {
                    return Err(AgentSessionError::NoOutstandingRequest);
                }
                Ok(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::Approval(decision),
                })
            },
            AgentIntent::RespondToUserInput { request, input } => {
                if !state.outstanding_requests.contains(&request) {
                    return Err(AgentSessionError::NoOutstandingRequest);
                }
                Ok(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::UserInput(UserInput::new(input)),
                })
            },
        }
    }

    fn queue_command(
        &mut self,
        command: AgentCommand,
        state: &mut SessionState,
    ) -> Result<CommandAdmission, AgentSessionError> {
        match &command {
            AgentCommand::StartTurn { turn, .. } => match state.active_turn {
                None => {
                    state.active_turn = Some(*turn);
                    state.turn_started = false;
                    self.active_turn_id
                        .store(turn.turn_id().get().get(), Ordering::Release);
                },
                Some(active) if active == *turn => {},
                Some(_) => return Err(AgentSessionError::TurnNoLongerActive),
            },
            AgentCommand::SteerTurn { turn, .. } | AgentCommand::InterruptTurn { turn }
                if state.active_turn != Some(*turn) =>
            {
                return Err(AgentSessionError::TurnNoLongerActive);
            },
            _ => {},
        }
        let response_request = match &command {
            AgentCommand::RespondToActivity { request, .. } => Some(*request),
            _ => None,
        };
        if response_request.is_some_and(|request| !state.outstanding_requests.contains(&request)) {
            return Err(AgentSessionError::NoOutstandingRequest);
        }
        if matches!(
            command,
            AgentCommand::SteerTurn { turn, .. }
                if state.interrupted_turns.contains(&turn)
        ) {
            return Err(AgentSessionError::TurnInterruptPending);
        }
        let interrupted = match &command {
            AgentCommand::InterruptTurn { turn } => Some(*turn),
            _ => None,
        };
        let urgent = response_request.is_some()
            || matches!(
                command,
                AgentCommand::InterruptTurn { turn }
                    if state.turn_started && state.active_turn == Some(turn)
            );
        let sender = if urgent {
            &self.urgent_commands
        } else {
            &self.commands
        };
        match sender.try_send(command) {
            Ok(()) => {
                if let Some(turn) = interrupted {
                    state.interrupted_turns.insert(turn);
                }
                if let Some(request) = response_request {
                    state.outstanding_requests.remove(&request);
                }
                Ok(CommandAdmission::Accepted)
            },
            Err(TrySendError::Full(command)) => Ok(CommandAdmission::Backpressured(
                PendingCommand::from_command(command),
            )),
            Err(TrySendError::Disconnected(_)) => Err(AgentSessionError::WorkerUnavailable(
                "the runtime worker closed before accepting an action".to_owned(),
            )),
        }
    }

    fn next_turn(&mut self) -> Result<TurnRef, AgentSessionError> {
        let id = NonZeroU64::new(self.next_turn_id).ok_or(AgentSessionError::TurnIdExhausted)?;
        self.next_turn_id = self.next_turn_id.checked_add(1).unwrap_or(0);
        Ok(TurnRef::new(self.session_id, TurnId::new(id)))
    }

    fn resolve_snapshot(&mut self, action: AgentIntent) -> Result<AgentCommand, AgentSessionError> {
        let active_turn = NonZeroU64::new(self.active_turn_id.load(Ordering::Acquire))
            .map(|id| TurnRef::new(self.session_id, TurnId::new(id)));
        match action {
            AgentIntent::Submit(text) => {
                let input = UserInput::new(text);
                if let Some(turn) = active_turn {
                    Ok(AgentCommand::SteerTurn { turn, input })
                } else {
                    Ok(AgentCommand::StartTurn {
                        turn: self.next_turn()?,
                        input,
                    })
                }
            },
            AgentIntent::Interrupt => Ok(AgentCommand::InterruptTurn {
                turn: active_turn.ok_or(AgentSessionError::NoActiveTurn)?,
            }),
            AgentIntent::RespondToApproval { request, decision } => {
                Ok(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::Approval(decision),
                })
            },
            AgentIntent::RespondToUserInput { request, input } => {
                Ok(AgentCommand::RespondToActivity {
                    request,
                    response: ActivityResponse::UserInput(UserInput::new(input)),
                })
            },
        }
    }

    /// Queues one frontend intent without waiting for provider acceptance.
    pub fn dispatch(&mut self, action: AgentIntent) -> Result<CommandAdmission, AgentSessionError> {
        let state = Arc::clone(&self.state);
        let mut state = match state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                let command = self.resolve_snapshot(action)?;
                return Ok(CommandAdmission::Backpressured(
                    PendingCommand::from_command(command),
                ));
            },
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        let command = self.resolve_action(action, &mut state)?;
        self.queue_command(command, &mut state)
    }

    /// Retries an operation retained by an earlier dispatch attempt.
    pub fn retry(
        &mut self,
        pending: PendingCommand,
    ) -> Result<CommandAdmission, AgentSessionError> {
        let command = pending.into_command();
        let state = Arc::clone(&self.state);
        let mut state = match state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => {
                return Ok(CommandAdmission::Backpressured(
                    PendingCommand::from_command(command),
                ));
            },
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        self.queue_command(command, &mut state)
    }
}
