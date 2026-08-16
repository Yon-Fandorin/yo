use super::super::CompleteModelBinding;

const COMPLETE_BINDING: &str = r#"{"provider":"qwencloud","account":"default","model":"model","connector":"openai-responses","base_url":"https://example.test/v1","api_dialect":"openai-responses","tokenizer_profile":"utf8-bytes/v1","input_token_limit":1000,"max_output_tokens":100,"reasoning_parameters":{"effort":"medium"},"optional_request_parameters":{},"tool_capability_policy":"local-tools/v1","verification_profile":"semantic-terminal/v1"}"#;

// complete binding equality는 좌표뿐 아니라 resolved profile 전부를 포함하므로 profile만
// 달라져도 새 binding epoch로 판정합니다.
#[test]
fn complete_binding_equality_includes_the_resolved_profile() {
    let medium = CompleteModelBinding::from_durable_json(COMPLETE_BINDING).unwrap();
    let high = CompleteModelBinding::from_durable_json(
        &COMPLETE_BINDING.replace(r#""effort":"medium""#, r#""effort":"high""#),
    )
    .unwrap();

    assert_eq!(medium.binding(), high.binding());
    assert_ne!(medium, high);
}

// 기존 durable complete binding의 replay_profile 부재는 semantic-only로만 해석되며,
// 같은 연결 좌표라도 Kimi private profile을 명시하면 완전한 binding identity가 달라집니다.
#[test]
fn replay_profile_omission_is_semantic_and_private_profile_changes_identity() {
    let semantic = CompleteModelBinding::from_durable_json(COMPLETE_BINDING).unwrap();
    let private = CompleteModelBinding::from_durable_json(&COMPLETE_BINDING.replace(
        r#""verification_profile":"semantic-terminal/v1""#,
        r#""verification_profile":"semantic-terminal/v1","replay_profile":"kimi-private-local-plaintext/v1""#,
    ))
    .unwrap();

    assert_eq!(
        semantic.profile().replay_profile().as_str(),
        "semantic-only/v1"
    );
    assert_eq!(semantic.binding(), private.binding());
    assert_ne!(semantic, private);
}

// durable reader는 부재와 명시적 semantic-only를 같은 의미로 읽되, null·unknown·duplicate는
// omission으로 축약하지 않고 exact private profile도 별도 binding identity로 보존합니다.
#[test]
fn complete_binding_replay_profile_is_presence_aware_and_closed() {
    let explicit_semantic = CompleteModelBinding::from_durable_json(&COMPLETE_BINDING.replace(
        r#""verification_profile":"semantic-terminal/v1""#,
        r#""verification_profile":"semantic-terminal/v1","replay_profile":"semantic-only/v1""#,
    ))
    .unwrap();
    assert_eq!(
        explicit_semantic.profile().replay_profile().as_str(),
        "semantic-only/v1"
    );

    for field in [
        r#","replay_profile":null"#,
        r#","replay_profile":"unknown/v1""#,
        r#","replay_profile":"kimi-private-local-plaintext/v1","replay_profile":"kimi-private-local-plaintext/v1""#,
    ] {
        let value = COMPLETE_BINDING.replace(
            r#""verification_profile":"semantic-terminal/v1""#,
            &format!(r#""verification_profile":"semantic-terminal/v1"{field}"#),
        );
        assert!(
            CompleteModelBinding::from_durable_json(&value).is_err(),
            "{field}"
        );
    }
}

// durable decoder는 serde_json이 범위 밖 정수를 float로 재분류하기 전에 lexical variant를
// 검사하고, 유한하지 않은 exponent도 frontend와 무관하게 core에서 거절합니다.
#[test]
fn complete_binding_decoder_rejects_out_of_range_number_spellings() {
    for invalid in [
        "18446744073709551616",
        "340282366920938463463374607431768211456",
        "1e400",
    ] {
        let value = COMPLETE_BINDING.replace(
            r#""reasoning_parameters":{"effort":"medium"}"#,
            &format!(r#""reasoning_parameters":{{"value":{invalid}}}"#),
        );
        let error = CompleteModelBinding::from_durable_json(&value).unwrap_err();
        assert!(error.to_string().contains("out-of-range number"), "{error}");
    }

    let quoted = COMPLETE_BINDING.replace(
        r#""reasoning_parameters":{"effort":"medium"}"#,
        r#""reasoning_parameters":{"value":"1e400"}"#,
    );
    CompleteModelBinding::from_durable_json(&quoted).unwrap();
}
