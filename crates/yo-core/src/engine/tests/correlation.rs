use super::{activity, engine_with_active_turn, id};
use crate::{
    ActivityKind, ActivityOutcome, ActivityRequestRef, ActivityResponse, AgentCommand, AgentEvent,
    AgentRejection, ApprovalDecision, ExpectedResponse, RequestId, ResponseKind, UserInput,
};

// 승인 요청에는 승인 응답만 허용하고 잘못된 종류의 응답이 요청을 소비하지 않는지 확인한다.
#[test]
fn correlates_and_type_checks_activity_responses() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let request_activity = activity(active_turn, 1);
    let request_id = RequestId::new(id(1));
    let request = ActivityRequestRef::new(request_activity, request_id);
    engine
        .start_activity(
            request_activity,
            ActivityKind::ApprovalRequest { request_id },
        )
        .unwrap();
    engine
        .finish_activity(request_activity, ActivityOutcome::Completed)
        .unwrap();

    let mismatch = engine
        .handle_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::UserInput(UserInput::from("yes")),
        })
        .unwrap_err();
    let accepted = engine
        .handle_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Approved),
        })
        .unwrap();
    let duplicate = engine
        .handle_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::Approval(ApprovalDecision::Declined),
        })
        .unwrap_err();

    assert_eq!(
        mismatch,
        AgentRejection::ResponseKindMismatch {
            request,
            expected: ExpectedResponse::Approval,
            actual: ResponseKind::UserInput,
        }
    );
    assert!(accepted.is_empty());
    assert_eq!(
        duplicate,
        AgentRejection::RequestAlreadyAnswered { request }
    );
}

// 응답 Activity가 실제 요청과 응답 명령 뒤에 한 번만 생기도록 백엔드 이벤트의 상관관계도 검증하는지
// 확인한다.
#[test]
fn records_one_correlated_response_activity_after_the_answer() {
    let (mut engine, active_turn) = engine_with_active_turn();
    let request_activity = activity(active_turn, 1);
    let response_activity = activity(active_turn, 2);
    let duplicate_response = activity(active_turn, 3);
    let request_id = RequestId::new(id(1));
    let request = ActivityRequestRef::new(request_activity, request_id);
    engine
        .start_activity(
            request_activity,
            ActivityKind::UserInputRequest { request_id },
        )
        .unwrap();

    let too_early = engine
        .start_activity(
            response_activity,
            ActivityKind::UserInputResponse { request_id },
        )
        .unwrap_err();
    engine
        .handle_command(AgentCommand::RespondToActivity {
            request,
            response: ActivityResponse::UserInput(UserInput::from("continue")),
        })
        .unwrap();
    let recorded = engine
        .start_activity(
            response_activity,
            ActivityKind::UserInputResponse { request_id },
        )
        .unwrap();
    let duplicate = engine
        .start_activity(
            duplicate_response,
            ActivityKind::UserInputResponse { request_id },
        )
        .unwrap_err();

    assert_eq!(too_early, AgentRejection::ResponseNotAnswered { request });
    assert_eq!(
        recorded,
        AgentEvent::ActivityStarted {
            activity: response_activity,
            kind: ActivityKind::UserInputResponse { request_id },
        }
    );
    assert_eq!(
        duplicate,
        AgentRejection::ResponseAlreadyRecorded { request }
    );
}
