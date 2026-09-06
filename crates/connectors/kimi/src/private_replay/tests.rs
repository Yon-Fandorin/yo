use super::{KimiAssistantMessage, KimiAssistantToolCall, decode_envelope, encode_envelope};

// Kimi crate가 private wire의 exact member order와 required/nullable 구분을 소유하고,
// core의 opaque envelope는 정상 payload bytes를 재정렬하지 않습니다.
#[test]
fn private_envelope_preserves_exact_stop_and_tool_payload_bytes() {
    let stop = encode_envelope(&KimiAssistantMessage::new(
        "private reasoning",
        Some("visible".to_owned()),
        Vec::new(),
    ))
    .unwrap();
    assert_eq!(
        stop.payload(),
        br#"{"role":"assistant","reasoning_content":"private reasoning","content":"visible"}"#
    );
    assert!(decode_envelope(&stop).is_ok());

    let tool = encode_envelope(&KimiAssistantMessage::new(
        "private reasoning",
        None,
        vec![KimiAssistantToolCall::new("call-1", "read_file", "{}")],
    ))
    .unwrap();
    assert_eq!(
        tool.payload(),
        br#"{"role":"assistant","reasoning_content":"private reasoning","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}"}}]}"#
    );
    assert!(decode_envelope(&tool).is_ok());
}

// omission, explicit null, empty arrays, duplicate outer/nested members는 서로 다른 wire
// 상태이므로 canonicalization 전에 fail-closed 합니다.
#[test]
fn private_envelope_rejects_presence_and_duplicate_member_ambiguity() {
    for malformed in [
        br#"{"role":"assistant","reasoning_content":"hidden"}"#.as_slice(),
        br#"{"role":"assistant","reasoning_content":"hidden","content":null}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":null}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[]}"#,
        br#"{"role":"user","reasoning_content":"hidden","content":"visible"}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":"visible","unknown":true}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"hosted","function":{"name":"read_file","arguments":"{}"}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"","type":"function","function":{"name":"read_file","arguments":"{}"}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"x","arguments":"{}"}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{"}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}","unknown":true}}]}"#,
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}"}},{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{}"}}]}"#,
    ] {
        let envelope = yo_core::ProviderPrivateReplayEnvelope::new(
            "kimi.assistant-message/v1alpha1",
            malformed.to_vec(),
        )
        .unwrap();
        assert!(decode_envelope(&envelope).is_err());
    }

    for duplicate in [
        br#"{"role":"assistant","role":"assistant","reasoning_content":"hidden","content":"visible"}"#.as_slice(),
        br#"{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","name":"write_file","arguments":"{}"}}]}"#,
    ] {
        assert!(
            yo_core::ProviderPrivateReplayEnvelope::new(
                "kimi.assistant-message/v1alpha1",
                duplicate.to_vec(),
            )
            .is_err()
        );
    }

    let wrong_schema = yo_core::ProviderPrivateReplayEnvelope::new(
        "other.assistant/v1",
        br#"{"role":"assistant","reasoning_content":"hidden","content":"visible"}"#.to_vec(),
    )
    .unwrap();
    assert!(decode_envelope(&wrong_schema).is_err());

    let mixed = encode_envelope(&KimiAssistantMessage::new(
        "hidden",
        Some("visible".to_owned()),
        vec![KimiAssistantToolCall::new("call-1", "read_file", "{}")],
    ))
    .unwrap();
    let decoded = decode_envelope(&mixed).unwrap();
    assert_eq!(decoded.content(), Some("visible"));
    assert_eq!(decoded.tool_calls().len(), 1);
}

// 복구한 private payload도 여러 call의 argument 합계가 exact 4 MiB만 허용하는지 검증합니다.
#[test]
fn private_envelope_enforces_the_aggregate_argument_boundary_on_decode() {
    let argument = |bytes: usize| format!("\"{}\"", "x".repeat(bytes - 2));
    let payload = |second_bytes: usize| {
        let first = serde_json::to_string(&argument(2 * 1024 * 1024)).unwrap();
        let second = serde_json::to_string(&argument(second_bytes)).unwrap();
        format!(
            r#"{{"role":"assistant","reasoning_content":"hidden","content":null,"tool_calls":[{{"id":"call-1","type":"function","function":{{"name":"read_file","arguments":{first}}}}},{{"id":"call-2","type":"function","function":{{"name":"read_file","arguments":{second}}}}}]}}"#,
        )
        .into_bytes()
    };

    let exact = yo_core::ProviderPrivateReplayEnvelope::new(
        "kimi.assistant-message/v1alpha1",
        payload(2 * 1024 * 1024),
    )
    .unwrap();
    assert!(decode_envelope(&exact).is_ok());

    let overflow = yo_core::ProviderPrivateReplayEnvelope::new(
        "kimi.assistant-message/v1alpha1",
        payload(2 * 1024 * 1024 + 1),
    )
    .unwrap();
    assert!(decode_envelope(&overflow).is_err());
}
