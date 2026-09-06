mod failure;
mod flow;
mod interaction;
mod journal;
mod replacement_binding;

use std::num::NonZeroU64;

use crate::{
    ActivityId, ActivityRef, AgentCommand, AgentRuntime, BackendScriptStep, ScriptedBackend,
    SessionId, SubmissionId, TurnId, TurnRef, UserInput,
};

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn session(value: u64) -> SessionId {
    crate::fixture_session(value)
}

fn turn(session_id: SessionId, value: u64) -> TurnRef {
    TurnRef::new(session_id, TurnId::new(id(value)))
}

fn activity(turn: TurnRef, value: u64) -> ActivityRef {
    ActivityRef::new(turn, ActivityId::new(id(value)))
}

fn submission(value: u8) -> SubmissionId {
    SubmissionId::from_uuid(uuid::Builder::from_random_bytes([value; 16]).into_uuid())
        .expect("the test submission fixture is a UUIDv4")
}

fn runtime_with_active_turn(
    later_steps: impl IntoIterator<Item = BackendScriptStep>,
) -> (AgentRuntime<ScriptedBackend>, TurnRef) {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let create = AgentCommand::CreateSession { session_id };
    let start = AgentCommand::StartTurn {
        turn,
        input: UserInput::from("inspect"),
    };
    let steps = [
        vec![
            BackendScriptStep::AcceptCommand(create.clone()),
            BackendScriptStep::AcceptCommand(start.clone()),
        ],
        later_steps.into_iter().collect(),
    ]
    .concat();
    let mut runtime = AgentRuntime::new(ScriptedBackend::new(steps));
    runtime.execute_command(create).unwrap();
    runtime.execute_submission(start, submission(1)).unwrap();
    (runtime, turn)
}
