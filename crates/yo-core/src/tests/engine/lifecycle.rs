use super::{activity, engine_with_active_turn, id, session, turn};
use crate::{
    ActivityKind, ActivityOutcome, ActivityResponse, AgentCommand, AgentEngine, AgentEvent,
    AgentRejection, ApprovalDecision, Failure, RequestId, TurnOutcome, UserInput,
};

// 세션과 Turn을 시작하면 상태가 보존되고 프런트엔드가 그대로 전달할 상관관계 이벤트가 생성되는지
// 확인한다.
#[test]
fn creates_a_session_and_starts_its_first_turn() {
    let session_id = session(1);
    let first_turn = turn(session_id, 1);
    let mut engine = AgentEngine::new();

    let created = engine
        .handle_command(AgentCommand::CreateSession { session_id })
        .unwrap();
    let started = engine
        .handle_command(AgentCommand::StartTurn {
            turn: first_turn,
            input: UserInput::from("inspect"),
        })
        .unwrap();

    assert_eq!(created, vec![AgentEvent::SessionCreated { session_id }]);
    assert_eq!(started, vec![AgentEvent::TurnStarted { turn: first_turn }]);
    assert_eq!(engine.session_id(), Some(session_id));
    assert_eq!(engine.active_turn(), Some(first_turn));
    assert_eq!(
        engine.active_turn_input().map(UserInput::as_str),
        Some("inspect")
    );
    assert_eq!(engine.turn_count(), 1);
}

// Activity의 시작·증분·종료가 같은 참조를 유지하고 종료 뒤 추가 증분은 거절되는지 확인한다.
#[test]
fn tracks_an_activity_through_its_lifecycle() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let activity = activity(active_turn, 1);

    let started = engine
        .start_activity(activity, ActivityKind::AgentMessage)
        .unwrap();
    let updated = engine
        .update_activity(
            activity,
            crate::ActivityUpdate::TextDelta("working".to_owned()),
        )
        .unwrap();
    let finished = engine
        .finish_activity(activity, ActivityOutcome::Completed)
        .unwrap();
    let rejection = engine
        .update_activity(
            activity,
            crate::ActivityUpdate::TextDelta("late".to_owned()),
        )
        .unwrap_err();

    assert_eq!(
        started,
        AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        }
    );
    assert_eq!(
        updated,
        AgentEvent::ActivityUpdated {
            activity,
            update: crate::ActivityUpdate::TextDelta("working".to_owned()),
        }
    );
    assert_eq!(
        finished,
        AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        }
    );
    assert_eq!(rejection, AgentRejection::ActivityNotActive { activity });
}

// 정상 완료는 요청 응답 명령과 상관관계 응답 Activity가 모두 끝난 뒤에만 허용하는지 단계별로
// 확인한다.
#[test]
fn completes_a_turn_only_after_its_request_response_cycle_closes() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let request_activity = activity(active_turn, 1);
    let response_activity = activity(active_turn, 2);
    let request_id = RequestId::new(id(1));
    let request = crate::ActivityRequestRef::new(request_activity, request_id);
    engine
        .start_activity(
            request_activity,
            ActivityKind::ApprovalRequest { request_id },
        )
        .unwrap();
    engine
        .finish_activity(request_activity, ActivityOutcome::Completed)
        .unwrap();

    let unanswered = engine
        .finish_turn(active_turn, TurnOutcome::Completed)
        .unwrap_err();
    engine
        .handle_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();
    let not_recorded = engine
        .finish_turn(active_turn, TurnOutcome::Completed)
        .unwrap_err();
    engine
        .start_activity(
            response_activity,
            ActivityKind::ApprovalResponse { request_id },
        )
        .unwrap();
    engine
        .finish_activity(response_activity, ActivityOutcome::Completed)
        .unwrap();
    let finished = engine
        .finish_turn(active_turn, TurnOutcome::Completed)
        .unwrap();

    assert_eq!(
        unanswered,
        AgentRejection::RequestStillUnanswered { request }
    );
    assert_eq!(
        not_recorded,
        AgentRejection::ResponseNotRecorded { request }
    );
    assert_eq!(
        finished,
        AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Completed,
        }
    );
    assert_eq!(engine.active_turn(), None);
}

// 실패 종료는 백엔드 단절처럼 응답을 받을 수 없는 상황을 표현하므로 미해결 요청을 남기고도 닫히는지
// 확인한다.
#[test]
fn failed_turn_may_abandon_an_unanswered_request() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let request_activity = activity(active_turn, 1);
    let request_id = RequestId::new(id(1));
    engine
        .start_activity(
            request_activity,
            ActivityKind::ApprovalRequest { request_id },
        )
        .unwrap();
    engine
        .finish_activity(request_activity, ActivityOutcome::Completed)
        .unwrap();
    let failure = Failure::new("backend disconnected");

    let finished = engine
        .finish_turn(active_turn, TurnOutcome::Failed(failure.clone()))
        .unwrap();

    assert_eq!(
        finished,
        AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Failed(failure),
        }
    );
    assert_eq!(engine.active_turn(), None);
}

// InterruptTurn은 요청만 기록하고 backend가 Activity와 Turn의 실제 중단을 알린 뒤에야 닫히는지
// 확인한다.
#[test]
fn interruption_waits_for_backend_terminal_events() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let first = activity(active_turn, 1);
    let already_finished = activity(active_turn, 2);
    engine
        .start_activity(first, ActivityKind::ModelWork)
        .unwrap();
    engine
        .start_activity(already_finished, ActivityKind::ToolResult)
        .unwrap();
    engine
        .finish_activity(already_finished, ActivityOutcome::Completed)
        .unwrap();

    let immediate = engine
        .handle_command(AgentCommand::InterruptTurn { turn: active_turn })
        .unwrap();
    assert!(immediate.is_empty());
    assert_eq!(engine.active_turn(), Some(active_turn));

    let activity_finished = engine
        .finish_activity(first, ActivityOutcome::Interrupted)
        .unwrap();
    let turn_finished = engine
        .finish_turn(active_turn, TurnOutcome::Interrupted)
        .unwrap();

    assert_eq!(
        activity_finished,
        AgentEvent::ActivityFinished {
            activity: first,
            outcome: ActivityOutcome::Interrupted,
        }
    );
    assert_eq!(
        turn_finished,
        AgentEvent::TurnFinished {
            turn: active_turn,
            outcome: TurnOutcome::Interrupted,
        }
    );
    assert_eq!(engine.active_turn(), None);
}
