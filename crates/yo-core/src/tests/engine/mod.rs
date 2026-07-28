mod correlation;
mod lifecycle;
mod rejection;

use std::num::NonZeroU64;

use crate::{
    ActivityId, ActivityRef, AgentCommand, AgentEngine, SessionId, TurnId, TurnRef, UserInput,
};

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn session(value: u64) -> SessionId {
    SessionId::new(id(value))
}

fn turn(session_id: SessionId, value: u64) -> TurnRef {
    TurnRef::new(session_id, TurnId::new(id(value)))
}

fn activity(turn: TurnRef, value: u64) -> ActivityRef {
    ActivityRef::new(turn, ActivityId::new(id(value)))
}

fn engine_with_active_turn() -> (AgentEngine, TurnRef) {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let mut engine = AgentEngine::new();
    engine
        .handle_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    engine
        .handle_command(AgentCommand::StartTurn {
            turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();
    (engine, turn)
}
