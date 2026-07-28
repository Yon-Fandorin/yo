use std::num::NonZeroU64;

use crate::{ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionId, TurnId, TurnRef};

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

// TurnRef가 어느 세션의 어느 턴인지 함께 보존해 암묵적인 현재 세션에 의존하지 않음을 확인한다.
#[test]
fn turn_reference_keeps_explicit_session_and_turn_identity() {
    let session_id = SessionId::new(id(1));
    let turn_id = TurnId::new(id(2));
    let turn = TurnRef::new(session_id, turn_id);

    assert_eq!(turn.session_id(), session_id);
    assert_eq!(turn.turn_id(), turn_id);
}

// ActivityRef 하나만 전달해도 세션·턴·활동의 전체 소속 관계를 복원할 수 있음을 확인한다.
#[test]
fn activity_reference_keeps_its_complete_owner_path() {
    let session_id = SessionId::new(id(1));
    let turn_id = TurnId::new(id(2));
    let activity_id = ActivityId::new(id(3));
    let activity = ActivityRef::new(TurnRef::new(session_id, turn_id), activity_id);

    assert_eq!(activity.session_id(), session_id);
    assert_eq!(activity.turn_id(), turn_id);
    assert_eq!(activity.activity_id(), activity_id);
}

// Activity 응답 대상이 원래 Activity와 요청 상관관계 ID를 함께 보존함을 확인한다.
#[test]
fn activity_request_reference_keeps_target_and_correlation_identity() {
    let activity = ActivityRef::new(
        TurnRef::new(SessionId::new(id(1)), TurnId::new(id(2))),
        ActivityId::new(id(3)),
    );
    let request_id = RequestId::new(id(4));
    let request = ActivityRequestRef::new(activity, request_id);

    assert_eq!(request.activity(), activity);
    assert_eq!(request.request_id(), request_id);
}
