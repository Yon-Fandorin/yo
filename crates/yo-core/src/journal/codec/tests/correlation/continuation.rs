use super::{
    super::{JournalRecord, decode, encode, recover},
    support::{
        model_replay_delta, valid_history, valid_history_with_profile_and_replay,
        valid_history_with_replay,
    },
};
use crate::{
    JournalSequence, ModelReplayContract, ModelReplayDelta, ModelReplayItem, ModelReplayRole,
    ProviderPrivateReplayEnvelope, ReplayProfile,
};

fn private_envelope(payload: &[u8]) -> ProviderPrivateReplayEnvelope {
    ProviderPrivateReplayEnvelope::new("kimi.assistant-message/v1alpha1", payload.to_vec()).unwrap()
}

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

// private replay profile과 같은 epoch의 Kimi assistant 물리 항목은 binding_epoch와 완전한
// message bytes를 보존하고 semantic recovery 뒤에도 최신 Anchor를 구성합니다.
#[test]
fn round_trips_private_assistant_only_under_the_private_replay_profile() {
    let replay_delta = private_replay_delta();
    let commits = valid_history_with_profile_and_replay(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        replay_delta.clone(),
    );
    let encoded = commits
        .iter()
        .map(encode)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let wire: serde_json::Value = serde_json::from_str(&encoded[3]).unwrap();
    assert_eq!(wire["records"][1]["items"][2]["binding_epoch"], 1);
    assert_eq!(
        wire["records"][1]["items"][2]["message"]["reasoning_content"],
        "private reasoning"
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
    assert_eq!(recovered.model_replay().items(), replay_delta.items());
}

// Journal은 provider payload를 해석하지 않고 stop/tool object의 exact physical shape을
// opaque bytes로 보존하며, profile/schema correlation만 검증합니다.
#[test]
fn private_assistant_wire_preserves_opaque_stop_and_tool_shapes() {
    let stop = valid_history_with_profile_and_replay(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        private_replay_delta(),
    )
    .pop()
    .unwrap();
    let encoded_stop = encode(&stop).unwrap();
    let stop_wire: serde_json::Value = serde_json::from_str(&encoded_stop).unwrap();
    assert!(
        stop_wire["records"][1]["items"][2]["message"]
            .get("tool_calls")
            .is_none()
    );
    decode(&encoded_stop).expect("absent tool_calls is the canonical stop shape");

    let tool = valid_history_with_profile_and_replay(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        private_tool_replay_delta(),
    )
    .pop()
    .unwrap();
    let encoded_tool = encode(&tool).unwrap();
    let tool_wire: serde_json::Value = serde_json::from_str(&encoded_tool).unwrap();
    assert!(tool_wire["records"][1]["items"][2]["message"]["content"].is_null());
    assert_eq!(
        tool_wire["records"][1]["items"][2]["message"]["tool_calls"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    decode(&encoded_tool).expect("one nonempty private tool call remains readable");
}

// duplicate private member의 이름이나 값이 Journal decode 오류에 새지 않는지 검증합니다.
#[test]
fn private_payload_duplicate_key_errors_never_echo_the_member_name() {
    let completion = valid_history_with_profile_and_replay(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        private_replay_delta(),
    )
    .pop()
    .unwrap();
    let encoded = encode(&completion).unwrap();
    let malformed = encoded.replace(
        r#""reasoning_content":"private reasoning""#,
        r#""private-secret-sentinel":1,"private-secret-sentinel":2,"reasoning_content":"private reasoning""#,
    );
    let error = decode(&malformed).unwrap_err();
    for rendered in [format!("{error}"), format!("{error:?}")] {
        assert!(!rendered.contains("private-secret-sentinel"), "{rendered}");
    }
}

// private item이 semantic-only epoch에 들어가거나 private epoch가 해당 item 없이 완료되면
// durable bytes가 유효해 보여도 profile 경계를 넘는 재생으로 Anchor를 만들지 않습니다.
#[test]
fn replay_profile_and_private_items_must_match_in_both_directions() {
    let semantic_error = recover(&valid_history_with_replay(private_replay_delta())).unwrap_err();
    assert!(semantic_error.to_string().contains("semantic-only"));

    let private_error = recover(&valid_history_with_profile_and_replay(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        model_replay_delta(),
    ))
    .unwrap_err();
    assert!(
        private_error
            .to_string()
            .contains("requires a provider-private")
    );
}

// earlier valid private가 뒤 assistant group의 누락을 가리지 못하도록 group마다 하나를 요구합니다.
#[test]
fn private_recovery_requires_one_private_item_after_every_assistant_group() {
    let replay_delta = ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: String::new(),
                refusal: None,
            },
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{"path":"README.md"}"#.to_owned(),
            },
            ModelReplayItem::ProviderPrivateAssistant {
                envelope: private_envelope(
                    br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"README.md\"}"}}]}"#,
                ),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "contents".to_owned(),
            },
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "final".to_owned(),
                refusal: None,
            },
        ],
    );
    let error = recover(&valid_history_with_profile_and_replay(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        replay_delta,
    ))
    .unwrap_err();
    assert!(
        error.to_string().contains("every assistant group"),
        "{error}"
    );
}

// orphan private와 assistant-call-private 순서를 벗어난 forged replay를 recovery가 거부합니다.
#[test]
fn private_recovery_rejects_orphan_and_pre_call_private_ordering() {
    let cases = [
        vec![ModelReplayItem::ProviderPrivateAssistant {
            envelope: private_envelope(br#"{"opaque":true}"#),
        }],
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: String::new(),
                refusal: None,
            },
            ModelReplayItem::ProviderPrivateAssistant {
                envelope: private_envelope(br#"{"opaque":true}"#),
            },
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: "{}".to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "contents".to_owned(),
            },
        ],
    ];
    for items in cases {
        let replay_delta =
            ModelReplayDelta::new(Some(ModelReplayContract::new("system", Vec::new())), items);
        let error = recover(&valid_history_with_profile_and_replay(
            ReplayProfile::ProviderPrivateLocalPlaintext,
            replay_delta,
        ))
        .unwrap_err();
        assert!(
            error.to_string().contains("provider-private replay")
                || error.to_string().contains("provider-private assistant"),
            "{error}"
        );
    }
}

// private replay profile은 exact Kimi envelope schema 이외의 Journal item을 받지 않습니다.
#[test]
fn private_replay_profile_rejects_a_wrong_envelope_schema_during_recovery() {
    let replay_delta = ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "visible".to_owned(),
                refusal: None,
            },
            ModelReplayItem::ProviderPrivateAssistant {
                envelope: ProviderPrivateReplayEnvelope::new(
                    "other.private/v1",
                    br#"{"opaque":true}"#.to_vec(),
                )
                .unwrap(),
            },
        ],
    );
    let commits = valid_history_with_profile_and_replay(
        ReplayProfile::ProviderPrivateLocalPlaintext,
        replay_delta,
    );
    let error = recover(&commits).unwrap_err();
    assert!(error.to_string().contains("schema"), "{error}");
}

// physical v1 exact replay은 field 부재만 semantic-only로 해석하고, 명시된 값은 exact
// private profile 하나뿐이므로 null·explicit semantic·unknown·duplicate를 거부합니다.
#[test]
fn exact_replay_profile_wire_is_presence_aware_and_canonical() {
    let encoded = encode(&valid_history()[1]).unwrap();
    for malformed in [
        serde_json::Value::Null,
        serde_json::json!("semantic-only/v1"),
        serde_json::json!("unknown/v1"),
    ] {
        let mut wire: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        wire["records"][1]["continuation_strategy"]["replay_profile"] = malformed;
        assert!(decode(&wire.to_string()).is_err());
    }

    let duplicate = encoded.replace(
        r#""executor":"local_client""#,
        r#""executor":"local_client","replay_profile":"kimi-private-local-plaintext/v1","replay_profile":"kimi-private-local-plaintext/v1""#,
    );
    assert!(decode(&duplicate).is_err());
}

fn private_replay_delta() -> ModelReplayDelta {
    ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::User,
                content: "hello".to_owned(),
                refusal: None,
            },
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: "visible".to_owned(),
                refusal: None,
            },
            ModelReplayItem::ProviderPrivateAssistant {
                envelope: private_envelope(
                    br#"{"role":"assistant","reasoning_content":"private reasoning","content":"visible"}"#,
                ),
            },
        ],
    )
}

fn private_tool_replay_delta() -> ModelReplayDelta {
    ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: String::new(),
                refusal: None,
            },
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: "{}".to_owned(),
            },
            ModelReplayItem::ProviderPrivateAssistant {
                envelope: private_envelope(
                    br#"{"role":"assistant","reasoning_content":"private reasoning","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}"}}]}"#,
                ),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "contents".to_owned(),
            },
        ],
    )
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
