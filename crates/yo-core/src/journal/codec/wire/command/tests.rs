use super::{WireActivityResponse, WireCommand};
use crate::{ActivityResponse, SecretInput};

// 실제 비밀 값은 영속화 wire 문법으로 변환할 수 없는지 확인한다.
#[test]
fn live_secret_input_is_outside_the_wire_grammar() {
    let secret = ActivityResponse::SecretInput(SecretInput::new("never persist me").unwrap());

    let error = match WireActivityResponse::try_from(&secret) {
        Ok(_) => panic!("a live secret must not enter the wire grammar"),
        Err(error) => error,
    };

    assert_eq!(
        error.to_string(),
        "live secret input is outside the persistence grammar"
    );
    assert!(!error.to_string().contains("never persist me"));
}

// 비밀 제출 영수증은 값 없는 단일 폐쇄형 wire 형태만 허용하는지 확인한다.
#[test]
fn secret_receipt_has_one_closed_payload_free_shape() {
    let response = WireActivityResponse::try_from(&ActivityResponse::SecretInputSubmitted).unwrap();
    let bytes = serde_json::to_string(&response).unwrap();

    assert_eq!(bytes, r#"{"type":"secret_input_submitted"}"#);
    assert!(matches!(
        serde_json::from_str::<WireActivityResponse>(&bytes).unwrap(),
        WireActivityResponse::SecretInputSubmitted {}
    ));
    assert!(
        serde_json::from_str::<WireActivityResponse>(
            r#"{"type":"secret_input_submitted","value":"forbidden"}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<WireCommand>(
            r#"{"type":"respond_to_activity","request":{"activity":{"turn":{"session_id":"00000000-0000-4000-8000-000000000001","turn_id":1},"activity_id":1},"request_id":1},"response":{"type":"secret_input","value":"forbidden"}}"#
        )
        .is_err()
    );
}
