use std::{
    num::NonZeroU64,
    thread,
    time::{Duration, Instant},
};

use yo_core::{
    ActivityId, ActivityKind, ActivityRef, ActivityUpdate, AgentCommand, AgentEvent, AgentIntent,
    BackendCapabilities, BackendEvent, BackendFailure, BackendFailureKind, BackendScriptStep,
    CommandAdmission, ScriptedBackend, SessionId, TranscriptRecord, TurnId, TurnOutcome, TurnRef,
    UserInput,
};
use yo_tui::{AgentConnection, AgentPoll, TerminationEvent, TerminationSource};

use super::TuiAgentConnection;

struct NeverTerminated;

impl TerminationSource for NeverTerminated {
    fn poll_termination(&mut self) -> TerminationEvent {
        TerminationEvent::None
    }
}

fn session_id() -> SessionId {
    SessionId::new(NonZeroU64::MIN)
}

fn turn() -> TurnRef {
    TurnRef::new(session_id(), TurnId::new(NonZeroU64::MIN))
}

fn collect_until(
    connection: &mut TuiAgentConnection,
    done: impl Fn(&[TranscriptRecord]) -> bool,
) -> Result<Vec<TranscriptRecord>, String> {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut records = Vec::new();
    loop {
        match connection.poll() {
            Ok(AgentPoll::Record(record)) => {
                records.push(record);
                if done(&records) {
                    return Ok(records);
                }
            },
            Ok(AgentPoll::Pending) if Instant::now() < deadline => thread::yield_now(),
            Ok(other) => {
                return Err(format!(
                    "connection ended before the expected records: {other:?}"
                ));
            },
            Err(error) => return Err(error.to_string()),
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for committed records".to_owned());
        }
    }
}

// 로컬 CLI 어댑터는 변경 알림 자체를 화면에 보내지 않고 저널에서 명령과 이벤트를
// 확정된 순서대로 읽어 TUI 레코드로 전달한다.
#[test]
fn exposes_committed_commands_and_events_in_journal_order() {
    let create = AgentCommand::CreateSession {
        session_id: session_id(),
    };
    let start = AgentCommand::StartTurn {
        turn: turn(),
        input: UserInput::from("inspect"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::AcceptCommand(start.clone()),
        BackendScriptStep::Emit(yo_core::BackendEvent::TurnFinished {
            turn: turn(),
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start(backend, &mut termination)
        .unwrap()
        .unwrap();

    assert_eq!(
        connection
            .dispatch(AgentIntent::Submit("inspect".to_owned()))
            .unwrap(),
        CommandAdmission::Accepted
    );
    let records = collect_until(&mut connection, |records| {
        records.iter().any(|record| {
            matches!(
                record,
                TranscriptRecord::EventCommitted(AgentEvent::TurnFinished { .. })
            )
        })
    })
    .unwrap();

    assert_eq!(
        records,
        [
            TranscriptRecord::CommandCommitted(create),
            TranscriptRecord::EventCommitted(AgentEvent::SessionCreated {
                session_id: session_id(),
            }),
            TranscriptRecord::CommandCommitted(start),
            TranscriptRecord::EventCommitted(AgentEvent::TurnStarted { turn: turn() }),
            TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                turn: turn(),
                outcome: TurnOutcome::Completed,
            }),
        ]
    );
    connection.shutdown().unwrap();
}

// frontend가 읽기 전에 한 알림으로 합쳐진 레코드가 한 번의 저널 읽기 상한인 256개를
// 넘어도, CLI 어댑터는 새 알림을 기다리지 않고 당시 head까지 모든 페이지를 이어 읽는다.
#[test]
fn drains_more_than_one_bounded_page_from_one_coalesced_wake() {
    const UPDATE_COUNT: usize = 260;
    const EXPECTED_RECORDS: usize = UPDATE_COUNT + 5;

    let activity = ActivityRef::new(turn(), ActivityId::new(NonZeroU64::new(1).unwrap()));
    let mut script = vec![
        BackendScriptStep::AcceptCommand(AgentCommand::CreateSession {
            session_id: session_id(),
        }),
        BackendScriptStep::AcceptCommand(AgentCommand::StartTurn {
            turn: turn(),
            input: UserInput::from("inspect"),
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        }),
    ];
    script.extend((0..UPDATE_COUNT).map(|index| {
        BackendScriptStep::Emit(BackendEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextSnapshot(index.to_string()),
        })
    }));
    script.push(BackendScriptStep::Shutdown(Ok(())));

    let mut termination = NeverTerminated;
    let mut connection = TuiAgentConnection::start(ScriptedBackend::new(script), &mut termination)
        .unwrap()
        .unwrap();
    connection
        .dispatch(AgentIntent::Submit("inspect".to_owned()))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(4);
    while connection
        .transcript
        .head_sequence()
        .is_none_or(|head| head.get() < EXPECTED_RECORDS as u64)
    {
        assert!(
            Instant::now() < deadline,
            "worker did not commit the complete coalesced Journal suffix"
        );
        thread::yield_now();
    }

    let records =
        collect_until(&mut connection, |records| records.len() == EXPECTED_RECORDS).unwrap();

    assert_eq!(records.len(), EXPECTED_RECORDS);
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(
                record,
                TranscriptRecord::EventCommitted(AgentEvent::ActivityUpdated { .. })
            ))
            .count(),
        UPDATE_COUNT
    );
    connection.shutdown().unwrap();
}

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
    let mut connection = TuiAgentConnection::start(backend, &mut termination)
        .unwrap()
        .unwrap();
    connection
        .dispatch(AgentIntent::Submit("inspect".to_owned()))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut saw_failed_turn = false;
    loop {
        match connection.poll() {
            Ok(AgentPoll::Record(TranscriptRecord::EventCommitted(AgentEvent::TurnFinished {
                outcome: TurnOutcome::Failed(_),
                ..
            }))) => saw_failed_turn = true,
            Ok(AgentPoll::Record(_)) | Ok(AgentPoll::Pending) => {},
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
    let mut connection = TuiAgentConnection::start(backend, &mut termination)
        .unwrap()
        .unwrap();
    connection
        .dispatch(AgentIntent::Submit("inspect".to_owned()))
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
        .dispatch(AgentIntent::Submit("focus".to_owned()))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut records = Vec::new();
    loop {
        match connection.poll() {
            Ok(AgentPoll::Record(record)) => records.push(record),
            Ok(AgentPoll::Pending) => {},
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
