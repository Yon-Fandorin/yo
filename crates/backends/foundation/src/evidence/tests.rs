use super::*;

fn private_envelope(payload: &[u8]) -> ProviderPrivateReplayEnvelope {
    ProviderPrivateReplayEnvelope::new("provider.private/v1", payload.to_vec()).unwrap()
}

// 중립 envelope가 versioned schema와 canonical bounded object만 허용하는지 검증합니다.
#[test]
fn provider_private_envelope_closes_schema_and_payload_grammar() {
    for schema in [
        "",
        "x",
        "provider.private/v",
        "provider/private/v1",
        "비공개/v1",
    ] {
        assert!(ProviderPrivateReplayEnvelope::new(schema, b"{}".to_vec()).is_err());
    }
    let oversized_schema = format!("{}/v1", "x".repeat(126));
    assert_eq!(oversized_schema.len(), 129);
    assert!(ProviderPrivateReplayEnvelope::new(oversized_schema, b"{}".to_vec()).is_err());

    for payload in [
        Vec::new(),
        b"[]".to_vec(),
        b"not-json".to_vec(),
        vec![0xff],
        br#"{ "member":1}"#.to_vec(),
        br#"{"outer":{"secret-sentinel":1,"secret-sentinel":2}}"#.to_vec(),
    ] {
        let error = ProviderPrivateReplayEnvelope::new("provider.private/v1", payload).unwrap_err();
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("secret-sentinel"), "{rendered}");
    }
}

// opaque payload의 generic exact ceiling은 받고 첫 초과 byte는 거부하는지 검증합니다.
#[test]
fn provider_private_envelope_accepts_the_exact_payload_ceiling_only() {
    let payload = |bytes: usize| {
        let mut payload = br#"{"x":""}"#.to_vec();
        payload.splice(6..6, std::iter::repeat_n(b'x', bytes - 8));
        assert_eq!(payload.len(), bytes);
        payload
    };

    assert!(
        ProviderPrivateReplayEnvelope::new("provider.private/v1", payload(MAX_REPLAY_TEXT_BYTES),)
            .is_ok()
    );
    assert!(
        ProviderPrivateReplayEnvelope::new(
            "provider.private/v1",
            payload(MAX_REPLAY_TEXT_BYTES + 1),
        )
        .is_err()
    );
}

// 리플레이 함수 결과는 앞선 정확한 호출과 한 번만 짝지어져야 한다.
#[test]
fn model_replay_rejects_missing_and_duplicate_function_relationships() {
    let mut replay = ModelReplay::default();
    let missing = ModelReplayDelta::new(
        None,
        vec![ModelReplayItem::FunctionCallOutput {
            call_id: "call-1".to_owned(),
            output: "missing".to_owned(),
        }],
    );
    assert!(replay.apply(&missing).is_err());

    let duplicate = ModelReplayDelta::new(
        None,
        vec![
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: "{}".to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "one".to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "two".to_owned(),
            },
        ],
    );
    assert!(replay.apply(&duplicate).is_err());
}

// 유효한 호출과 결과는 원래 인자·출력 바이트를 바꾸지 않고 누적된다.
#[test]
fn model_replay_preserves_one_exact_function_relationship() {
    let delta = ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: r#"{ "path": "README.md" }"#.to_owned(),
            },
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "contents".to_owned(),
            },
        ],
    );
    let mut replay = ModelReplay::default();
    replay.apply(&delta).unwrap();
    assert_eq!(replay.items(), delta.items());
}

// visible refusal은 assistant가 낸 관찰에만 의미가 있으므로 system·developer·user 역할에
// 붙은 replay는 저장이나 다음 dialect 직렬화 전에 공통 증거 경계에서 거부한다.
#[test]
fn model_replay_rejects_refusal_on_non_assistant_messages() {
    for role in [
        ModelReplayRole::System,
        ModelReplayRole::Developer,
        ModelReplayRole::User,
    ] {
        let delta = ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            vec![ModelReplayItem::Message {
                role,
                content: String::new(),
                refusal: Some("declined".to_owned()),
            }],
        );

        assert!(!delta.is_valid());
    }
}

// replay contract·delta·전체 prefix가 각 바이트 상한을 넘으면 저장 전에 거부하는지 검증합니다.
#[test]
fn model_replay_enforces_contract_delta_and_prefix_byte_bounds() {
    let oversized_contract =
        ModelReplayContract::new("x".repeat(MAX_REPLAY_CONTRACT_BYTES), Vec::new());
    assert!(!oversized_contract.is_valid());

    let oversized_delta = ModelReplayDelta::new(
        None,
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "x".repeat(MAX_REPLAY_DELTA_BYTES),
            refusal: None,
        }],
    );
    assert!(!oversized_delta.is_valid());

    let body = "x".repeat(13 * 1024 * 1024);
    let mut replay = ModelReplay::default();
    for index in 0..4 {
        replay
            .apply(&ModelReplayDelta::new(
                (index == 0).then(|| ModelReplayContract::new("system", Vec::new())),
                vec![ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: body.clone(),
                    refusal: None,
                }],
            ))
            .unwrap();
    }
    let error = replay
        .apply(&ModelReplayDelta::new(
            None,
            vec![ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: body,
                refusal: None,
            }],
        ))
        .unwrap_err();
    assert!(error.contains("prefix byte limit"));
}

// contract만 있고 semantic item이 없는 delta는 완료 Turn의 replay 증거가 될 수 없음을 검증합니다.
#[test]
fn model_replay_delta_requires_at_least_one_semantic_item() {
    assert!(
        !ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            Vec::new(),
        )
        .is_valid()
    );
}

// replay contract은 chain의 첫 delta에 정확히 한 번만 나타나야 하며 누락·지연 선언을 거부하는지
// 검증합니다.
#[test]
fn model_replay_contract_is_required_on_the_first_delta_only() {
    let message = || ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: "answer".to_owned(),
        refusal: None,
    };
    let mut replay = ModelReplay::default();
    assert!(
        replay
            .apply(&ModelReplayDelta::new(None, vec![message()]))
            .unwrap_err()
            .contains("first")
    );

    replay
        .apply(&ModelReplayDelta::new(
            Some(ModelReplayContract::new("system", Vec::new())),
            vec![message()],
        ))
        .unwrap();
    assert!(
        replay
            .apply(&ModelReplayDelta::new(
                Some(ModelReplayContract::new("late", Vec::new())),
                vec![message()],
            ))
            .unwrap_err()
            .contains("more than once")
    );
}

// provider-private payload는 foundation의 public Debug 경계를 거쳐도 길이만 남고 원문은
// 노출되지 않습니다.
#[test]
fn provider_private_debug_is_redacted_through_public_enclosing_types() {
    let private = "private-reasoning-sentinel";
    let envelope = private_envelope(br#"{"private":"private-reasoning-sentinel"}"#);
    let replay = ModelReplayItem::ProviderPrivateAssistant {
        envelope: envelope.clone(),
    };
    let delta = ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", Vec::new())),
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: String::new(),
                refusal: None,
            },
            replay.clone(),
        ],
    );

    for rendered in [
        format!("{envelope:?}"),
        format!("{replay:?}"),
        format!("{delta:?}"),
    ] {
        assert!(!rendered.contains(private), "{rendered}");
        assert!(rendered.contains("payload_bytes"));
    }
}

// opaque private item도 core가 확인할 수 있는 assistant/call 인접성은 지켜야 합니다.
#[test]
fn provider_private_item_requires_one_preceding_visible_assistant_group() {
    let private = || ModelReplayItem::ProviderPrivateAssistant {
        envelope: private_envelope(br#"{"opaque":true}"#),
    };
    let assistant = || ModelReplayItem::Message {
        role: ModelReplayRole::Assistant,
        content: String::new(),
        refusal: None,
    };
    let user = || ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: "input".to_owned(),
        refusal: None,
    };
    let contract = || Some(ModelReplayContract::new("system", Vec::new()));

    for items in [
        vec![private()],
        vec![user(), private()],
        vec![assistant(), private(), private()],
        vec![
            ModelReplayItem::Message {
                role: ModelReplayRole::Assistant,
                content: String::new(),
                refusal: Some("declined".to_owned()),
            },
            private(),
        ],
    ] {
        assert!(ModelReplayDelta::new(contract(), items).validate().is_err());
    }

    let valid = ModelReplayDelta::new(
        contract(),
        vec![
            assistant(),
            ModelReplayItem::FunctionCall {
                call_id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: "{}".to_owned(),
            },
            private(),
            ModelReplayItem::FunctionCallOutput {
                call_id: "call-1".to_owned(),
                output: "contents".to_owned(),
            },
        ],
    );
    assert!(valid.validate().is_ok());
}
