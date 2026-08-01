use super::{
    super::{AgentIntent, AgentSession},
    support::{session, turn},
};
use crate::{
    AgentCommand, AgentEvent, BackendScriptStep, ScriptedBackend, TranscriptRecord, TurnOutcome,
    UserInput,
};

// frontend가 변경 알림을 한 번도 poll하지 않아도 Journal reader는 Worker가 의미적으로
// commit한 명령과 이벤트를 순서대로 읽어야 Transcript replay의 독립된 근거가 된다.
#[test]
fn reads_worker_commits_without_consuming_the_change_notification() {
    let first = turn(1);
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: first,
            input: UserInput::from("inspect"),
        }),
        BackendScriptStep::Emit(crate::BackendEvent::TurnFinished {
            turn: first,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut app = AgentSession::start_for_test(backend, session()).unwrap();
    let transcript = app.transcript_reader();
    let startup = transcript.read_after(None);
    let startup_cursor = startup.entries().last().unwrap().sequence();

    app.dispatch(AgentIntent::Submit("inspect".to_owned()))
        .unwrap();
    app.wait_until_processed(1);
    app.wait_until_no_active_turn();

    let records = transcript
        .read_after(Some(startup_cursor))
        .into_entries()
        .into_iter()
        .map(|entry| entry.record().clone())
        .collect::<Vec<_>>();
    assert_eq!(
        records,
        vec![
            TranscriptRecord::CommandCommitted(AgentCommand::StartTurn {
                turn: first,
                input: UserInput::from("inspect"),
            }),
            TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { turn: first }),
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                turn: first,
                outcome: TurnOutcome::Completed,
            }),
        ]
    );

    app.shutdown().unwrap();
}
