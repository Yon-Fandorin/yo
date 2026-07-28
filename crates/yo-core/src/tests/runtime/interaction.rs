use super::{activity, id, runtime_with_active_turn, session, turn};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, AgentCommand, AgentEvent,
    AgentRejection, AgentRuntime, ApprovalDecision, BackendCapabilities, BackendEvent,
    BackendScriptStep, RequestId, RuntimeError, RuntimePoll, ScriptedBackend, UserInput,
};

// approval 요청과 사용자 응답이 steer나 새 Turn으로 바뀌지 않고 하나의 상관관계 흐름으로
// 정상 완료되는지 확인한다.
#[test]
fn completes_one_correlated_approval_cycle() {
    let active_turn = turn(session(1), 1);
    let request_activity = activity(active_turn, 1);
    let response_activity = activity(active_turn, 2);
    let request_id = RequestId::new(id(1));
    let request = ActivityRequestRef::new(request_activity, request_id);
    let response_command = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::Approval(ApprovalDecision::Approved),
    };
    let steps = [
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: request_activity,
            kind: ActivityKind::ApprovalRequest { request_id },
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: request_activity,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::AcceptCommand(response_command.clone()),
        BackendScriptStep::Emit(BackendEvent::ActivityStarted {
            activity: response_activity,
            kind: ActivityKind::ApprovalResponse { request_id },
        }),
        BackendScriptStep::Emit(BackendEvent::ActivityFinished {
            activity: response_activity,
            outcome: ActivityOutcome::Completed,
        }),
        BackendScriptStep::Emit(BackendEvent::TurnFinished {
            turn: active_turn,
            outcome: crate::TurnOutcome::Completed,
        }),
        BackendScriptStep::Shutdown(Ok(())),
    ];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            kind: ActivityKind::ApprovalRequest { .. },
            ..
        })
    ));
    runtime.poll_event().unwrap();
    assert!(
        runtime
            .execute_command(response_command)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::ActivityStarted {
            kind: ActivityKind::ApprovalResponse { .. },
            ..
        })
    ));
    runtime.poll_event().unwrap();
    assert_eq!(
        runtime.poll_event().unwrap(),
        RuntimePoll::Event(AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: crate::TurnOutcome::Completed,
        })
    );
    assert_eq!(runtime.active_turn(), None);
    runtime.shutdown().unwrap();
}

// steer를 지원하지 않는 backend에서는 core가 명령을 명시적으로 거절하고 backend script를
// 소비하지 않는지 확인한다.
#[test]
fn unsupported_steer_never_reaches_the_backend() {
    let active_turn = turn(session(1), 1);
    let steps = [BackendScriptStep::Shutdown(Ok(()))];
    let (mut runtime, _) = runtime_with_active_turn(steps);

    let error = runtime
        .execute_command(AgentCommand::SteerTurn {
            turn: active_turn,
            input: UserInput::from("change direction"),
        })
        .unwrap_err();

    assert_eq!(
        error,
        RuntimeError::CommandRejected(AgentRejection::UnsupportedSteer)
    );
    assert_eq!(runtime.backend().remaining_steps(), 1);
    assert_eq!(runtime.active_turn(), Some(active_turn));
    runtime.shutdown().unwrap();
}

// steer capability가 확정된 backend에서는 같은 명령을 수락하되 Queue나 새 Turn을 만들지
// 않는지 확인한다.
#[test]
fn supported_steer_is_forwarded_without_creating_a_turn() {
    let session_id = session(1);
    let active_turn = turn(session_id, 1);
    let create = AgentCommand::CreateSession { session_id };
    let start = AgentCommand::StartTurn {
        turn: active_turn,
        input: UserInput::from("inspect"),
    };
    let steer = AgentCommand::SteerTurn {
        turn: active_turn,
        input: UserInput::from("focus on tests"),
    };
    let backend = ScriptedBackend::new([
        BackendScriptStep::AcceptCommand(create.clone()),
        BackendScriptStep::AcceptCommand(start.clone()),
        BackendScriptStep::AcceptCommand(steer.clone()),
        BackendScriptStep::Shutdown(Ok(())),
    ])
    .with_capabilities(BackendCapabilities::none().with_steer());
    let mut runtime = AgentRuntime::new(backend);
    runtime.execute_command(create).unwrap();
    runtime.execute_command(start).unwrap();

    let events = runtime.execute_command(steer).unwrap();

    assert!(events.is_empty());
    assert_eq!(runtime.active_turn(), Some(active_turn));
    assert_eq!(runtime.backend().remaining_steps(), 1);
    runtime.shutdown().unwrap();
}
