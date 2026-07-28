use std::num::NonZeroU64;

use crate::{
    ActivityId, ActivityKind, ActivityOutcome, ActivityRef, ActivityRequestRef, ActivityResponse,
    ActivityUpdate, AgentCommand, AgentEvent, ApprovalDecision, Failure, RequestId, SessionId,
    TurnId, TurnOutcome, TurnRef, UserInput,
};

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn references() -> (SessionId, TurnRef, ActivityRef) {
    let session_id = SessionId::new(id(1));
    let turn = TurnRef::new(session_id, TurnId::new(id(2)));
    let activity = ActivityRef::new(turn, ActivityId::new(id(3)));
    (session_id, turn, activity)
}

// 프런트엔드가 세션 내부를 직접 수정하지 않고 명시적인 도메인 명령으로 의도를 전달함을 확인한다.
#[test]
fn commands_carry_the_identity_of_their_target() {
    let (session_id, turn, _) = references();
    let input = UserInput::from("inspect the repository");

    let AgentCommand::CreateSession {
        session_id: observed_session,
    } = (AgentCommand::CreateSession { session_id })
    else {
        panic!("expected a create-session command");
    };
    let AgentCommand::StartTurn {
        turn: observed_turn,
        input: observed_input,
    } = (AgentCommand::StartTurn {
        turn,
        input: input.clone(),
    })
    else {
        panic!("expected a start-turn command");
    };

    assert_eq!(observed_session, session_id);
    assert_eq!(observed_turn, turn);
    assert_eq!(observed_input, input);
    assert_ne!(
        AgentCommand::InterruptTurn { turn },
        AgentCommand::InterruptTurn {
            turn: TurnRef::new(SessionId::new(id(9)), TurnId::new(id(9))),
        }
    );
}

// 활동의 시작·갱신·종료 이벤트가 모두 같은 ActivityRef를 가져 상관관계를 잃지 않음을 확인한다.
#[test]
fn every_activity_phase_carries_the_same_correlation_identity() {
    let (_, _, activity) = references();
    let events = [
        AgentEvent::ActivityStarted {
            activity,
            kind: ActivityKind::AgentMessage,
        },
        AgentEvent::ActivityUpdated {
            activity,
            update: ActivityUpdate::TextDelta("hello".to_owned()),
        },
        AgentEvent::ActivityFinished {
            activity,
            outcome: ActivityOutcome::Completed,
        },
    ];

    for event in events {
        let observed = match event {
            AgentEvent::ActivityStarted { activity, .. }
            | AgentEvent::ActivityUpdated { activity, .. }
            | AgentEvent::ActivityFinished { activity, .. } => activity,
            _ => panic!("expected an activity event"),
        };
        assert_eq!(observed, activity);
    }
}

// 승인 응답 명령과 응답 Activity가 원래 요청의 Activity와 request ID를 함께 가리킴을 확인한다.
#[test]
fn activity_response_keeps_its_request_correlation() {
    let (_, _, activity) = references();
    let request_id = RequestId::new(id(4));
    let request = ActivityRequestRef::new(activity, request_id);
    let command = AgentCommand::RespondToActivity {
        request,
        response: ActivityResponse::Approval(ApprovalDecision::Approved),
    };
    let event = AgentEvent::ActivityStarted {
        activity: ActivityRef::new(activity.turn(), ActivityId::new(id(5))),
        kind: ActivityKind::ApprovalResponse { request_id },
    };

    let AgentCommand::RespondToActivity {
        request: observed_request,
        ..
    } = command
    else {
        panic!("expected an activity-response command");
    };
    let AgentEvent::ActivityStarted {
        kind:
            ActivityKind::ApprovalResponse {
                request_id: observed_request_id,
            },
        ..
    } = event
    else {
        panic!("expected an approval-response activity");
    };

    assert_eq!(observed_request.activity(), activity);
    assert_eq!(observed_request.request_id(), request_id);
    assert_eq!(observed_request_id, request_id);
}

// Turn 종료가 정상 완료·사용자 중단·실패를 구분해 프런트엔드가 문자열을 해석하지 않아도 됨을
// 확인한다.
#[test]
fn turn_outcomes_distinguish_completion_interruption_and_failure() {
    let (_, turn, _) = references();
    let failure = Failure::new("backend disconnected");
    let outcomes = [
        TurnOutcome::Completed,
        TurnOutcome::Interrupted,
        TurnOutcome::Failed(failure.clone()),
    ];

    assert!(matches!(outcomes[0], TurnOutcome::Completed));
    assert!(matches!(outcomes[1], TurnOutcome::Interrupted));
    assert!(matches!(
        &outcomes[2],
        TurnOutcome::Failed(observed) if observed == &failure
    ));
    assert_ne!(outcomes[0], outcomes[1]);
    assert_ne!(outcomes[1], outcomes[2]);

    let AgentEvent::TurnFinished {
        turn: observed_turn,
        outcome: observed_outcome,
    } = (AgentEvent::TurnFinished {
        turn,
        outcome: outcomes[2].clone(),
    })
    else {
        panic!("expected a turn-finished event");
    };
    assert_eq!(observed_turn, turn);
    assert_eq!(observed_outcome, TurnOutcome::Failed(failure));
}
