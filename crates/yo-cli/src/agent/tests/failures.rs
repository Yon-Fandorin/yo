use std::{
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    AgentCommand, AgentEvent, AgentIntent, BackendCapabilities, BackendFailure, BackendFailureKind,
    BackendScriptStep, ScriptedBackend, TranscriptRecord, TurnOutcome, UserInput,
};
use yo_tui::{AgentConnection, AgentPoll};

use super::support::{NeverTerminated, collect_until, session_id, turn};
use crate::agent::TuiAgentConnection;

// 백엔드 실패가 저널에 Turn 실패를 확정한 경우 CLI 어댑터는 그 레코드를 먼저 모두
// 전달한 뒤 연결 오류를 보고한다.
#[test]
fn drains_committed_failure_record_before_reporting_connection_failure() {
    let create = AgentCommand::CreateSession {
        session_id: session_id(),
    };
    let start = AgentCommand::StartTurn {
        turn: turn(),
        input: UserInput::from("inspect"),
    };
    let backend_failure =
        BackendFailure::new(BackendFailureKind::ProcessExit, "provider stream stopped");
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create),
        BackendScriptStep::AcceptCommand(start),
        BackendScriptStep::Fail(backend_failure),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start(backend, session_id(), &mut termination)
        .unwrap()
        .unwrap();
    connection
        .dispatch(AgentIntent::submit("inspect".to_owned()).unwrap())
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut saw_failed_turn = false;
    loop {
        match connection.poll() {
            Ok(AgentPoll::Record(TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Failed(_),
                ..
            }))) => saw_failed_turn = true,
            Ok(
                AgentPoll::Record(_)
                | AgentPoll::Pending
                | AgentPoll::Submission(_)
                | AgentPoll::RequestTrace(_),
            ) => {},
            Ok(other) => panic!("connection closed without its failure: {other:?}"),
            Err(error) => {
                assert!(saw_failed_turn);
                assert!(error.to_string().contains("provider stream stopped"));
                break;
            },
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the connection failure"
        );
        thread::yield_now();
    }
    connection.shutdown().unwrap();
}

// 활성 Turn의 후속 명령이 backend에서 거절되면 worker는 cleanup이 확정한 Turn 종료를
// 먼저 공개하고, 그 다음에 명령 실패를 연결 오류로 보고한다.
#[test]
fn drains_cleanup_record_before_reporting_command_failure() {
    let create = AgentCommand::CreateSession {
        session_id: session_id(),
    };
    let start = AgentCommand::StartTurn {
        turn: turn(),
        input: UserInput::from("inspect"),
    };
    let steer = AgentCommand::SteerTurn {
        turn: turn(),
        input: UserInput::from("focus"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create),
        BackendScriptStep::AcceptCommand(start),
        BackendScriptStep::RejectCommand {
            command: steer.clone(),
            failure: BackendFailure::new(BackendFailureKind::Turn, "steer was rejected"),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ])
    .with_capabilities(BackendCapabilities::none().with_steer());
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start(backend, session_id(), &mut termination)
        .unwrap()
        .unwrap();
    connection
        .dispatch(AgentIntent::submit("inspect".to_owned()).unwrap())
        .unwrap();
    collect_until(&mut connection, |records| {
        records.iter().any(|record| {
            matches!(
                record,
                TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { .. })
            )
        })
    })
    .unwrap();
    connection
        .dispatch(AgentIntent::submit("focus".to_owned()).unwrap())
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut records = Vec::new();
    loop {
        match connection.poll() {
            Ok(AgentPoll::Record(record)) => records.push(record),
            Ok(AgentPoll::Pending | AgentPoll::Submission(_) | AgentPoll::RequestTrace(_)) => {},
            Ok(other) => panic!("connection closed without its failure: {other:?}"),
            Err(error) => {
                assert!(error.to_string().contains("steer was rejected"));
                break;
            },
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the command failure"
        );
        thread::yield_now();
    }

    assert!(!records.contains(&TranscriptRecord::CommandCommitted(steer)));
    assert!(records.iter().any(|record| {
        matches!(
            record,
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Interrupted,
                ..
            })
        )
    }));
    connection.shutdown().unwrap();
}
