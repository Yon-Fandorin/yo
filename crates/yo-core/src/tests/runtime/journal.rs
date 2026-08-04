use super::{activity, runtime_with_active_turn, session, submission, turn};
use crate::{
    ActivityKind, AgentCommand, AgentRuntime, BackendEvent, BackendFailure, BackendFailureKind,
    BackendScriptStep, RuntimeError, ScriptedBackend, UserInput, journal::SemanticRecord,
};

// Runtime의 공개 경계도 Start/Steer와 SubmissionId를 분리해서 받을 수 없게 막아,
// backend 호출 전에 Journal이 표현할 수 없는 command correlation을 거절합니다.
#[test]
fn rejects_a_runtime_command_with_the_wrong_submission_correlation_shape() {
    let session_id = session(1);
    let turn = turn(session_id, 1);
    let mut runtime =
        AgentRuntime::new(ScriptedBackend::new([BackendScriptStep::Shutdown(Ok(()))]));

    assert_eq!(
        runtime
            .execute_command(AgentCommand::StartTurn {
                turn,
                input: UserInput::new("missing identity"),
            })
            .unwrap_err(),
        RuntimeError::SubmissionIdentityRequired
    );
    assert_eq!(
        runtime
            .execute_submission(AgentCommand::CreateSession { session_id }, submission(10),)
            .unwrap_err(),
        RuntimeError::SubmissionIdentityUnexpected
    );
    assert!(runtime.journal().entries().is_empty());
    runtime.shutdown().unwrap();
}

// AgentSession을 우회해 공개 Runtime API를 직접 사용해도 이미 commit된 SubmissionId는
// backend와 Journal에 두 번째로 전달되지 않아 Session 단위 correlation이 유지됩니다.
#[test]
fn rejects_a_duplicate_submission_identity_at_the_runtime_boundary() {
    let (mut runtime, active_turn) =
        runtime_with_active_turn([BackendScriptStep::Shutdown(Ok(()))]);
    let before = runtime.journal().entries().len();

    let error = runtime
        .execute_submission(
            AgentCommand::SteerTurn {
                turn: active_turn,
                input: UserInput::new("duplicate"),
            },
            submission(1),
        )
        .expect_err("a committed SubmissionId cannot be reused");

    assert_eq!(
        error,
        RuntimeError::DuplicateSubmissionIdentity(submission(1))
    );
    assert_eq!(runtime.journal().entries().len(), before);
    assert_eq!(runtime.backend().remaining_steps(), 1);
    runtime.shutdown().unwrap();
}

// Runtime이 backend에서 수락되고 core에 commit된 명령만 Journal에 남겨야 하므로,
// 거절된 중단 명령은 기록하지 않고 뒤이어 수락된 같은 명령만 정확히 한 번 기록한다.
#[test]
fn records_only_commands_that_reach_semantic_commit() {
    let active_turn = turn(session(1), 1);
    let interrupt = AgentCommand::InterruptTurn { turn: active_turn };
    let failure = BackendFailure::new(BackendFailureKind::Turn, "interrupt rejected");
    let steps = [
        BackendScriptStep::RejectCommand {
            command: interrupt.clone(),
            failure: failure.clone(),
        },
        BackendScriptStep::AcceptCommand(interrupt.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);
    let before = runtime.journal().entries().len();

    assert_eq!(
        runtime.execute_command(interrupt.clone()).unwrap_err(),
        RuntimeError::Backend {
            failure,
            terminal_events: Vec::new(),
        }
    );
    assert_eq!(runtime.journal().entries().len(), before);

    assert!(
        runtime
            .execute_command(interrupt.clone())
            .unwrap()
            .is_empty()
    );
    assert_eq!(runtime.journal().entries().len(), before + 1);
    assert_eq!(
        runtime.journal().entries().last().unwrap().record(),
        &SemanticRecord::CommandCommitted(
            crate::journal::CommittedCommand::uncorrelated(interrupt).unwrap()
        )
    );
}

// backend event를 의미 상태에 commit한 뒤 frontend에 반환할 때 같은 AgentEvent가 Journal에도
// 먼저 남아야 이후 Transcript가 live 출력과 동일한 순서를 replay할 수 있다.
#[test]
fn records_committed_backend_events_before_they_are_observed() {
    let active_turn = turn(session(1), 1);
    let work = activity(active_turn, 1);
    let backend_event = BackendEvent::ActivityStarted {
        activity: work,
        kind: ActivityKind::ModelWork,
    };
    let steps = [
        BackendScriptStep::Emit(backend_event),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    let observed = runtime.poll_event().unwrap();

    let crate::RuntimePoll::Event(event) = observed else {
        panic!("expected one committed runtime event");
    };
    assert_eq!(
        runtime.journal().entries().last().unwrap().record(),
        &SemanticRecord::EventCommitted(event)
    );
}

// backend failure가 활성 Activity와 Turn을 닫을 때 frontend에 돌려주는 terminal event
// 전체가 같은 순서로 Journal에도 남아야 실패 직전의 history가 조용히 잘리지 않는다.
#[test]
fn records_terminal_events_created_by_backend_failure() {
    let active_turn = turn(session(1), 1);
    let work = activity(active_turn, 1);
    let failure = BackendFailure::new(BackendFailureKind::ProcessExit, "backend exited");
    let steps = [
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: work,
            kind: ActivityKind::ModelWork,
        }),
        BackendScriptStep::Fail(failure),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);
    runtime.poll_event().unwrap();
    let before_failure = runtime.journal().entries().len();

    let RuntimeError::Backend {
        terminal_events, ..
    } = runtime.poll_event().unwrap_err()
    else {
        panic!("expected a backend failure");
    };

    let recorded = &runtime.journal().entries()[before_failure..];
    assert!(
        !terminal_events.is_empty(),
        "an active Turn failure must create terminal events"
    );
    assert_eq!(recorded.len(), terminal_events.len());
    for (entry, event) in recorded.iter().zip(terminal_events) {
        assert_eq!(entry.record(), &SemanticRecord::EventCommitted(event));
    }
}
