use super::{activity, engine_with_active_turn, id, session, turn};
use crate::{
    ActivityKind, ActivityRequestRef, ActivityUpdate, AgentCommand, AgentRejection, RequestId,
    TurnOutcome, UserInput,
};

// 한 Session에서 활성 Turn이 둘 생기지 않도록 두 번째 StartTurn이 현재 활성 Turn을 명시하며
// 거절되는지 확인한다.
#[test]
fn rejects_a_second_concurrent_turn() {
    let (mut engine, active) = engine_with_active_turn();
    let attempted = turn(active.session_id(), 2);

    let rejection = engine
        .handle_command(AgentCommand::StartTurn {
            turn: attempted,
            input: UserInput::from("another request"),
        })
        .unwrap_err();

    assert_eq!(rejection, AgentRejection::TurnAlreadyActive { active });
    assert_eq!(engine.active_turn(), Some(active));
    assert_eq!(engine.turn_count(), 1);
}

// 이미 생성된 엔진에 다른 Session을 만들려 해도 기존 Session을 교체하지 않고 두 ID를 함께
// 보고하는지 확인한다.
#[test]
fn rejects_replacing_the_loaded_session() {
    let (mut engine, active) = engine_with_active_turn();
    let requested = session(2);

    let rejection = engine
        .handle_command(AgentCommand::CreateSession {
            session_id: requested,
        })
        .unwrap_err();

    assert_eq!(
        rejection,
        AgentRejection::SessionAlreadyExists {
            existing: active.session_id(),
            requested,
        }
    );
    assert_eq!(engine.session_id(), Some(active.session_id()));
    assert_eq!(engine.active_turn(), Some(active));
}

// 명령이 다른 Session ID를 가리키면 저장된 Session과 섞지 않고 기대값과 실제값을 함께 알려주는지
// 확인한다.
#[test]
fn rejects_a_turn_targeting_another_session() {
    let (mut engine, active) = engine_with_active_turn();
    engine.finish_turn(active, TurnOutcome::Completed).unwrap();
    let foreign_session = session(2);
    let foreign_turn = turn(foreign_session, 1);

    let rejection = engine
        .handle_command(AgentCommand::StartTurn {
            turn: foreign_turn,
            input: UserInput::from("wrong target"),
        })
        .unwrap_err();

    assert_eq!(
        rejection,
        AgentRejection::SessionMismatch {
            expected: active.session_id(),
            actual: foreign_session,
        }
    );
    assert_eq!(engine.turn_count(), 1);
}

// 이미 끝난 Turn ID를 재사용해 과거 기록을 덮어쓰지 못하도록 중복 시작을 거절하는지 확인한다.
#[test]
fn rejects_reusing_a_finished_turn_identity() {
    let (mut engine, finished) = engine_with_active_turn();
    engine
        .finish_turn(finished, TurnOutcome::Completed)
        .unwrap();

    let rejection = engine
        .handle_command(AgentCommand::StartTurn {
            turn: finished,
            input: UserInput::from("reuse"),
        })
        .unwrap_err();

    assert_eq!(rejection, AgentRejection::DuplicateTurn { turn: finished });
    assert_eq!(engine.turn_count(), 1);
}

// 같은 Activity ID나 request ID가 다시 들어와도 기존 활동과 응답 대상을 덮어쓰지 않는지 확인한다.
#[test]
fn rejects_duplicate_activity_and_request_identities() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let first = activity(active_turn, 1);
    let second = activity(active_turn, 2);
    let request_id = RequestId::new(id(1));
    engine
        .start_activity(first, ActivityKind::ApprovalRequest { request_id })
        .unwrap();

    let duplicate_activity = engine
        .start_activity(first, ActivityKind::AgentMessage)
        .unwrap_err();
    let duplicate_request = engine
        .start_activity(second, ActivityKind::UserInputRequest { request_id })
        .unwrap_err();

    assert_eq!(
        duplicate_activity,
        AgentRejection::DuplicateActivity { activity: first }
    );
    assert_eq!(
        duplicate_request,
        AgentRejection::DuplicateRequest {
            request: ActivityRequestRef::new(second, request_id),
        }
    );
    assert_eq!(
        engine
            .update_activity(second, ActivityUpdate::TextDelta("missing".to_owned()))
            .unwrap_err(),
        AgentRejection::ActivityNotActive { activity: second }
    );
}

// 활성 Activity가 남아 있으면 Turn 완료 이벤트를 먼저 만들지 못하게 해 수명주기 순서를 보장하는지
// 확인한다.
#[test]
fn requires_active_activities_to_finish_before_turn_completion() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let active_activity = activity(active_turn, 1);
    engine
        .start_activity(active_activity, ActivityKind::ToolCall)
        .unwrap();

    let rejection = engine
        .finish_turn(active_turn, TurnOutcome::Completed)
        .unwrap_err();

    assert_eq!(
        rejection,
        AgentRejection::ActivityStillActive {
            activity: active_activity,
        }
    );
    assert_eq!(engine.active_turn(), Some(active_turn));
}

// 백엔드가 Steer 지원을 선언하기 전에는 입력을 Queue나 새 Turn으로 바꾸지 않고 명시적으로 미지원
// 처리하는지 확인한다.
#[test]
fn rejects_steer_explicitly_until_a_backend_supports_it() {
    let (mut engine, active_turn) = engine_with_active_turn();

    let rejection = engine
        .handle_command(AgentCommand::SteerTurn {
            turn: active_turn,
            input: UserInput::from("change direction"),
        })
        .unwrap_err();

    assert_eq!(rejection, AgentRejection::UnsupportedSteer);
    assert_eq!(engine.active_turn(), Some(active_turn));
}

// Steer 미지원 여부를 알리기 전에 대상 Turn을 검증해 잘못된 상관관계가 미지원 오류로 가려지지
// 않는지 확인한다.
#[test]
fn validates_the_steer_target_before_reporting_unsupported() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let inactive = turn(active_turn.session_id(), 2);

    let rejection = engine
        .handle_command(AgentCommand::SteerTurn {
            turn: inactive,
            input: UserInput::from("wrong target"),
        })
        .unwrap_err();

    assert_eq!(rejection, AgentRejection::TurnNotActive { turn: inactive });
    assert_eq!(engine.active_turn(), Some(active_turn));
}
