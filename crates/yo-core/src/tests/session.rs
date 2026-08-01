use std::num::NonZeroU64;

use crate::{ActivityId, ActivityRef, ActivityRequestRef, RequestId, SessionId, TurnId, TurnRef};

fn id(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

// TurnRef가 어느 세션의 어느 턴인지 함께 보존해 암묵적인 현재 세션에 의존하지 않음을 확인한다.
#[test]
fn turn_reference_keeps_explicit_session_and_turn_identity() {
    let session_id = crate::fixture_session(1);
    let turn_id = TurnId::new(id(2));
    let turn = TurnRef::new(session_id, turn_id);

    assert_eq!(turn.session_id(), session_id);
    assert_eq!(turn.turn_id(), turn_id);
}

// ActivityRef 하나만 전달해도 세션·턴·활동의 전체 소속 관계를 복원할 수 있음을 확인한다.
#[test]
fn activity_reference_keeps_its_complete_owner_path() {
    let session_id = crate::fixture_session(1);
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
        TurnRef::new(crate::fixture_session(1), TurnId::new(id(2))),
        ActivityId::new(id(3)),
    );
    let request_id = RequestId::new(id(4));
    let request = ActivityRequestRef::new(activity, request_id);

    assert_eq!(request.activity(), activity);
    assert_eq!(request.request_id(), request_id);
}

// 새 Session ID가 매번 UUIDv7으로 생성되고 공개 문자열을 다시 parse해도 같은 값을
// 얻어야 저장 파일명과 향후 CLI 직접 선택이 하나의 정규 식별자를 공유할 수 있다.
#[test]
fn generated_session_identity_round_trips_as_uuidv7() {
    let first = SessionId::new().expect("the OS provides UUIDv7 inputs");
    let second = SessionId::new().expect("the OS provides UUIDv7 inputs");
    let encoded = first.to_string();

    assert!(first.as_uuid().is_some());
    assert_ne!(first, second);
    assert_eq!(encoded.parse::<SessionId>().unwrap(), first);
}

// UUID 문자열이 문법적으로 맞더라도 v4이면 Session ID로 받아들이지 않아 새 기록에
// 계약과 다른 UUID 버전이 조용히 섞이는 것을 막는다.
#[test]
fn rejects_a_non_v7_uuid_as_a_session_identity() {
    assert!(
        "67e55044-10b1-426f-9247-bb680e5fe0c8"
            .parse::<SessionId>()
            .is_err()
    );
}

// version nibble만 7인 NCS variant 값은 RFC UUIDv7이 아니므로 거부해야 외부 입력이
// 파일명과 wire identity에 표준 UUID처럼 저장되는 일을 막을 수 있다.
#[test]
fn rejects_a_non_rfc_variant_with_v7_version_bits() {
    assert!(
        "01890f00-0000-7000-0000-000000000001"
            .parse::<SessionId>()
            .is_err()
    );
}
