use super::{activity, runtime_with_active_turn, session, submission};
use crate::{
    ActivityKind, ActivityOutcome, AgentCommand, AgentEvent, AgentRejection, BackendEvent,
    BackendFailure, BackendFailureKind, BackendScriptStep, RuntimeError, TurnOutcome, UserInput,
};

// backend가 Session 생성 명령을 거절하면 core Session이 생성되지 않고 terminal event도
// 만들어지지 않는지 확인한다.
#[test]
fn command_rejection_leaves_core_state_unchanged() {
    let session_id = session(1);
    let create = AgentCommand::CreateSession { session_id };
    let failure = BackendFailure::new(BackendFailureKind::Session, "thread creation failed");
    let backend = crate::ScriptedBackend::new([
        BackendScriptStep::RejectCommand {
            command: create.clone(),
            failure: failure.clone(),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut runtime = crate::AgentRuntime::new(backend);

    let error = runtime.execute_command(create).unwrap_err();

    assert_eq!(
        error,
        RuntimeError::Backend {
            failure,
            terminal_events: Vec::new(),
        }
    );
    assert_eq!(runtime.session_id(), None);
    runtime.shutdown().unwrap();
}

// core에서 거절할 잘못된 명령은 backend script를 소비하지 않아 provider에 전달되지 않는지
// 확인한다.
#[test]
fn invalid_core_command_never_reaches_the_backend() {
    let expected_session = session(1);
    let expected = AgentCommand::CreateSession {
        session_id: expected_session,
    };
    let backend = crate::ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(expected.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ]);
    let mut runtime = crate::AgentRuntime::new(backend);
    let foreign_turn = super::turn(session(2), 1);

    let error = runtime
        .execute_submission(
            AgentCommand::StartTurn {
                turn: foreign_turn,
                input: UserInput::from("invalid"),
            },
            submission(5),
        )
        .unwrap_err();

    assert_eq!(
        error,
        RuntimeError::CommandRejected(AgentRejection::SessionNotCreated)
    );
    assert_eq!(runtime.backend().remaining_steps(), 2);
    runtime.execute_command(expected).unwrap();
    runtime.shutdown().unwrap();
}

// 활성 Turn의 중단 명령을 backend가 거절하면 core가 중단 요청을 기록하지 않아 같은 명령을
// 다시 보낼 수 있고, 두 번째 수락 뒤에도 실제 terminal event 전까지 Turn이 유지되는지 확인한다.
#[test]
fn rejected_interrupt_can_be_retried_without_mutating_the_turn() {
    let active_turn = super::turn(session(1), 1);
    let interrupt = AgentCommand::InterruptTurn { turn: active_turn };
    let rejection = BackendFailure::new(BackendFailureKind::Turn, "interrupt was not accepted");
    let steps = [
        BackendScriptStep::RejectCommand {
            command: interrupt.clone(),
            failure: rejection.clone(),
        },
        BackendScriptStep::AcceptCommand(interrupt.clone()),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Interrupted,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    assert_eq!(
        runtime.execute_command(interrupt.clone()).unwrap_err(),
        RuntimeError::Backend {
            failure: rejection,
            terminal_events: Vec::new(),
        }
    );
    assert_eq!(runtime.active_turn(), Some(active_turn));
    assert!(runtime.execute_command(interrupt).unwrap().is_empty());
    assert_eq!(runtime.active_turn(), Some(active_turn));
    assert_eq!(
        runtime.poll_event().unwrap(),
        crate::RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Interrupted,
        })
    );
    runtime.shutdown().unwrap();
}

// streaming backend failure가 활성 Activity와 Turn을 Failed로 닫는 terminal event를 함께
// 보존하는지 확인한다.
#[test]
fn backend_stream_failure_closes_the_active_semantic_state() {
    let active_turn = super::turn(session(1), 1);
    let work = activity(active_turn, 1);
    let failure = BackendFailure::new(
        BackendFailureKind::ProcessExit,
        "app-server exited unexpectedly",
    );
    let steps = [
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: work,
            kind: ActivityKind::ModelWork,
        }),
        BackendScriptStep::Fail(failure.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);
    runtime.poll_event().unwrap();

    let error = runtime.poll_event().unwrap_err();

    let RuntimeError::Backend {
        failure: observed,
        terminal_events,
    } = error
    else {
        panic!("expected a backend runtime failure");
    };
    assert_eq!(observed, failure);
    assert_eq!(
        terminal_events[0],
        AgentEvent::ActivityFinished {
            activity: work,
            outcome: ActivityOutcome::Failed(crate::Failure::new(
                "ProcessExit: app-server exited unexpectedly"
            )),
        }
    );
    assert!(matches!(
        &terminal_events[1],
        AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Failed(_),
        } if *turn == active_turn
    ));
    assert_eq!(runtime.active_turn(), None);
    runtime.shutdown().unwrap();
}

// provider가 Turn 실행 실패를 보고하면 ProcessExit와 구분되는 Turn failure를 유지하면서
// 활성 Turn을 Failed로 닫는지 확인한다.
#[test]
fn turn_failure_kind_remains_distinguishable() {
    let active_turn = super::turn(session(1), 1);
    let failure = BackendFailure::new(BackendFailureKind::Turn, "model execution failed");
    let steps = [
        BackendScriptStep::Fail(failure.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    let error = runtime.poll_event().unwrap_err();

    let RuntimeError::Backend {
        failure: observed,
        terminal_events,
    } = error
    else {
        panic!("expected a backend runtime failure");
    };
    assert_eq!(observed.kind(), BackendFailureKind::Turn);
    assert_eq!(observed, failure);
    assert!(matches!(
        terminal_events.as_slice(),
        [AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Failed(_),
        }] if *turn == active_turn
    ));
    assert_eq!(runtime.active_turn(), None);
    runtime.shutdown().unwrap();
}

// explicit shutdown의 cleanup 실패는 Cleanup failure를 보존하고 남아 있던 활성 Turn을
// Failed로 닫아, 성공한 종료나 예상 밖 process exit와 구분되는지 확인한다.
#[test]
fn cleanup_failure_closes_the_active_turn_as_failed() {
    let active_turn = super::turn(session(1), 1);
    let failure = BackendFailure::new(BackendFailureKind::Cleanup, "child reap failed");
    let steps = [BackendScriptStep::Shutdown(Err(failure.clone()))];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    let error = runtime.shutdown().unwrap_err();

    let RuntimeError::Backend {
        failure: observed,
        terminal_events,
    } = error
    else {
        panic!("expected a cleanup failure");
    };
    assert_eq!(observed.kind(), BackendFailureKind::Cleanup);
    assert_eq!(observed, failure);
    assert!(matches!(
        terminal_events.as_slice(),
        [AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Failed(_),
        }] if *turn == active_turn
    ));
    assert_eq!(runtime.active_turn(), None);
}

// 잘못된 backend 상관관계 event는 protocol 위반으로 격리하고 활성 Turn을 실패로 닫는지
// 확인한다.
#[test]
fn malformed_backend_event_is_rejected_and_fails_the_turn() {
    let active_turn = super::turn(session(1), 1);
    let unknown = activity(active_turn, 9);
    let steps = [
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: unknown,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    let error = runtime.poll_event().unwrap_err();

    let RuntimeError::EventRejected {
        rejection,
        terminal_events,
        ..
    } = error
    else {
        panic!("expected an event rejection");
    };
    assert_eq!(
        rejection,
        AgentRejection::ActivityNotActive { activity: unknown }
    );
    assert!(matches!(
        terminal_events.as_slice(),
        [AgentEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Failed(_),
        }] if *turn == active_turn
    ));
    assert_eq!(runtime.active_turn(), None);
    runtime.shutdown().unwrap();
}
