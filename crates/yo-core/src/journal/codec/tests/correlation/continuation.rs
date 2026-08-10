use super::{
    super::{JournalRecord, decode, encode, recover},
    support::{model_replay_delta, valid_history, valid_history_with_replay},
};
use crate::{
    JournalSequence, ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
};

// 완결된 Turn의 command·exchange·accepted request·outcome·Anchor를 encode/decode한 뒤에도
// recovery가 같은 열린 epoch와 최신 Anchor를 재구성해야 안전하게 native resume할 수 있습니다.
#[test]
fn round_trips_and_recovers_a_complete_continuation_chain() {
    let commits = valid_history()
        .into_iter()
        .map(|commit| decode(&encode(&commit).unwrap()).unwrap())
        .collect::<Vec<_>>();

    let recovered = recover(&commits).expect("the complete continuation chain recovers");

    assert_eq!(recovered.binding_epoch(), Some(1));
    assert_eq!(
        recovered.continuation_anchor(),
        Some(JournalSequence::new(9))
    );
    let replay = recovered
        .records()
        .iter()
        .find_map(|record| match record.record() {
            JournalRecord::ModelReplayDelta(replay) => Some(replay.delta()),
            _ => None,
        });
    assert_eq!(replay, Some(&model_replay_delta()));
}

// assistant refusal을 가진 exact replay delta는 물리 JSONL, semantic recovery, 최신 Anchor
// 재구성을 모두 통과하고 content와 refusal의 독립적인 바이트를 그대로 보존한다.
#[test]
fn round_trips_refusal_through_physical_recovery_and_anchor_validation() {
    let replay_delta = ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "visible".to_owned(),
            refusal: Some("declined".to_owned()),
        }],
    );
    let encoded = valid_history_with_replay(replay_delta.clone())
        .into_iter()
        .map(|commit| encode(&commit).unwrap())
        .collect::<Vec<_>>();
    let refusal_wire: serde_json::Value = serde_json::from_str(&encoded[3]).unwrap();
    assert_eq!(
        refusal_wire["records"][1]["items"][0]["refusal"],
        "declined"
    );

    let decoded = encoded
        .iter()
        .map(|commit| decode(commit).unwrap())
        .collect::<Vec<_>>();
    let recovered = recover(&decoded).unwrap();

    assert_eq!(
        recovered.continuation_anchor(),
        Some(JournalSequence::new(9))
    );
    let recovered_delta = recovered
        .records()
        .iter()
        .find_map(|record| match record.record() {
            JournalRecord::ModelReplayDelta(record) => Some(record.delta()),
            _ => None,
        });
    assert_eq!(recovered_delta, Some(&replay_delta));
}

// additive v1 reader는 직전 refusal-absent message를 계속 읽지만, 명시적 null이나
// non-string refusal은 필드 부재로 축약하지 않고 닫힌 wire 경계에서 거부한다.
#[test]
fn accepts_absent_refusal_but_rejects_null_and_non_string_values() {
    let completion = valid_history().pop().unwrap();
    let encoded = encode(&completion).unwrap();
    let absent: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert!(absent["records"][1]["items"][0].get("refusal").is_none());
    decode(&encoded).expect("the preceding refusal-absent shape remains readable");

    for malformed in [serde_json::Value::Null, serde_json::json!(7)] {
        let mut wire: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        wire["records"][1]["items"][0]["refusal"] = malformed;
        let error = decode(&wire.to_string()).expect_err("refusal must be a JSON string");
        assert!(error.to_string().contains("string"), "{error}");
    }
}

// exact replay outcome이 별도 delta record를 즉시 참조하지 않으면 decode가 실패하는지 검증합니다.
#[test]
fn exact_replay_requires_a_separate_immediately_referenced_delta() {
    let completion = valid_history().pop().unwrap();
    let mut wire: serde_json::Value = serde_json::from_str(&encode(&completion).unwrap()).unwrap();
    wire["records"][2]
        .as_object_mut()
        .unwrap()
        .remove("replay_delta_sequence");

    let error = decode(&wire.to_string()).expect_err("exact replay cannot omit its delta link");

    assert!(error.to_string().contains("referenced"), "{error}");
}

// backend-managed state에 Yo replay delta를 섞으면 strategy 계약 위반으로 recovery가 거부하는지
// 검증합니다.
#[test]
fn backend_managed_state_forbids_model_replay_delta() {
    let mut commits = valid_history();
    let opened = &mut commits[1];
    let mut wire: serde_json::Value = serde_json::from_str(&encode(opened).unwrap()).unwrap();
    wire["records"][1]["continuation_strategy"] =
        serde_json::json!({ "mode": "backend_managed_state" });
    commits[1] = decode(&wire.to_string()).unwrap();

    let error = recover(&commits).expect_err("backend-managed state cannot claim Yo replay");

    assert!(
        error.to_string().contains("exact-replay open epoch"),
        "{error}"
    );
}

// outcome 안에 replay payload를 넣던 이전 shape을 닫힌 wire schema가 다시 받아들이지 않는지
// 검증합니다.
#[test]
fn rejects_the_displaced_nested_replay_outcome_shape() {
    let completion = valid_history().pop().unwrap();
    let mut wire: serde_json::Value = serde_json::from_str(&encode(&completion).unwrap()).unwrap();
    wire["records"][2]["model_replay"] = serde_json::json!({
        "contract": null,
        "items": [{ "kind": "message", "role": "assistant", "content": "old" }]
    });

    let error = decode(&wire.to_string()).expect_err("the preceding /v1 shape must fail closed");

    assert!(error.to_string().contains("unknown field"), "{error}");
}
