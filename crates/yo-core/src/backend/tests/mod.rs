use std::num::NonZeroU64;

use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, AgentCommand, BackendAdapter,
    BackendCapabilities, BackendEvent, BackendFailure, BackendFailureKind, BackendPoll,
    BackendScriptStep, ScriptedBackend, SessionId, TurnId, TurnOutcome, TurnRef, UserInput,
};

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn references() -> (SessionId, TurnRef, ActivityRef) {
    let session_id = crate::fixture_session(1);
    let turn = TurnRef::new(session_id, TurnId::new(id(1)));
    let activity = ActivityRef::new(turn, ActivityId::new(id(1)));
    (session_id, turn, activity)
}

// 명령 기대값과 semantic event가 선언된 순서대로만 소비되고 다음 명령을 기다릴 때는 Pending을
// 반환하는지 확인한다.
#[test]
fn scripted_backend_replays_an_interleaved_flow_deterministically() {
    let (session_id, turn, activity) = references();
    let create = AgentCommand::CreateSession { session_id };
    let start = AgentCommand::StartTurn {
        turn,
        input: UserInput::from("inspect"),
    };
    let activity_started = BackendEvent::ActivityStarted {
        activity,
        kind: ActivityKind::ToolCall,
    };
    let mut backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::AcceptCommand(start.clone()),
        BackendScriptStep::Emit(activity_started.clone()),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn,
            outcome: TurnOutcome::Completed,
        }),
        BackendScriptStep::Close,
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    backend.execute_command(create).unwrap();
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    backend.execute_command(start).unwrap();
    assert_eq!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(activity_started)
    );
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::ActivityFinished { .. })
    ));
    assert!(matches!(
        backend.poll_event().unwrap(),
        BackendPoll::Event(BackendEvent::TurnFinished { .. })
    ));
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Closed);
    assert_eq!(backend.remaining_steps(), 1);
    backend.shutdown().unwrap();
    assert!(backend.is_exhausted());
}

// 예상과 다른 명령은 script를 소비하지 않아 진단 뒤 올바른 명령으로 같은 시나리오를 계속 검증할
// 수 있는지 확인한다.
#[test]
fn unexpected_command_is_explicit_and_non_consuming() {
    let (session_id, turn, _) = references();
    let expected = AgentCommand::CreateSession { session_id };
    let mut backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(expected.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    let failure = backend
        .execute_command(AgentCommand::InterruptTurn { turn })
        .unwrap_err();

    assert_eq!(failure.kind(), BackendFailureKind::Protocol);
    assert!(failure.message().contains("unexpected command"));
    assert_eq!(backend.remaining_steps(), 2);
    backend.execute_command(expected).unwrap();
    backend.shutdown().unwrap();
}

// 명령 실행 단계에서 발생한 Session 실패가 polling 실패와 섞이지 않고 선언된 종류 그대로
// 반환되며, 잘못된 선행 명령은 거절 단계를 소비하지 않는지 확인한다.
#[test]
fn command_rejection_is_typed_and_non_consuming() {
    let (session_id, turn, _) = references();
    let create = AgentCommand::CreateSession { session_id };
    let session_failure =
        BackendFailure::new(BackendFailureKind::Session, "thread creation failed");
    let mut backend = ScriptedBackend::new([
        BackendScriptStep::RejectCommand {
            command: create.clone(),
            failure: session_failure.clone(),
        },
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    let mismatch = backend
        .execute_command(AgentCommand::InterruptTurn { turn })
        .unwrap_err();
    assert_eq!(backend.remaining_steps(), 2);
    let rejection = backend.execute_command(create).unwrap_err();

    assert_eq!(mismatch.kind(), BackendFailureKind::Protocol);
    assert_eq!(rejection, session_failure);
    assert_eq!(backend.remaining_steps(), 1);
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Pending);
    backend.shutdown().unwrap();
}

// script의 failure가 종류와 메시지를 보존한 채 한 번 전달되고 이후 단계로 진행되는지 확인한다.
#[test]
fn scripted_failure_preserves_its_typed_category() {
    let failure = BackendFailure::new(
        BackendFailureKind::ProcessExit,
        "app-server exited unexpectedly",
    );
    let mut backend = ScriptedBackend::new([
        BackendScriptStep::Fail(failure.clone()),
        BackendScriptStep::Close,
        BackendScriptStep::Shutdown(Ok(())),
    ]);

    assert_eq!(backend.poll_event().unwrap_err(), failure);
    assert_eq!(backend.poll_event().unwrap(), BackendPoll::Closed);
    backend.shutdown().unwrap();
}

// cleanup 실패와 성공 모두 반복 호출에서 같은 결과를 반환해 상위 cleanup 경로가 안전하게 재시도할
// 수 있는지 확인한다.
#[test]
fn shutdown_is_idempotent_for_success_and_failure() {
    let cleanup_failure = BackendFailure::new(BackendFailureKind::Cleanup, "child did not exit");
    let mut successful = ScriptedBackend::new([BackendScriptStep::Shutdown(Ok(()))]);
    let mut failed =
        ScriptedBackend::new([BackendScriptStep::Shutdown(Err(cleanup_failure.clone()))]);

    assert_eq!(successful.shutdown(), Ok(()));
    assert_eq!(successful.shutdown(), Ok(()));
    assert_eq!(successful.poll_event().unwrap(), BackendPoll::Closed);

    assert_eq!(failed.shutdown(), Err(cleanup_failure.clone()));
    assert_eq!(failed.shutdown(), Err(cleanup_failure));
    assert_eq!(failed.poll_event().unwrap(), BackendPoll::Closed);
}

// 첫 coding loop가 구분해야 하는 backend 실패 단계가 서로 다른 안정된 값으로 유지되는지 확인한다.
#[test]
fn backend_failure_kinds_remain_distinguishable() {
    let kinds = [
        BackendFailureKind::Unavailable,
        BackendFailureKind::Initialization,
        BackendFailureKind::Session,
        BackendFailureKind::Unsupported,
        BackendFailureKind::Protocol,
        BackendFailureKind::ProcessExit,
        BackendFailureKind::Turn,
        BackendFailureKind::Cleanup,
    ];

    for (index, left) in kinds.iter().enumerate() {
        for right in &kinds[index + 1..] {
            assert_ne!(left, right);
        }
    }
}

// 초기화 때 확정된 steer capability가 fake에서도 provider-neutral 값으로 고정되어 노출되는지
// 확인한다.
#[test]
fn scripted_backend_exposes_fixed_capabilities() {
    let default_backend = ScriptedBackend::new([]);
    let steer_backend =
        ScriptedBackend::new([]).with_capabilities(BackendCapabilities::none().with_steer());

    assert!(!default_backend.capabilities().supports_steer());
    assert!(steer_backend.capabilities().supports_steer());
}
