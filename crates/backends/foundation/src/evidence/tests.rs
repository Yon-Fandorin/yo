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

// checkpoint root는 request-local delta의 16 MiB보다 클 수 있지만 전체 replay-prefix
// 64 MiB 상한과 공통 관계 검증은 그대로 적용해야 한다.
#[test]
fn context_checkpoint_replay_root_uses_the_prefix_budget_not_the_delta_budget() {
    let items = vec![
        ModelReplayItem::Message {
            role: ModelReplayRole::User,
            content: "x".repeat(9 * 1024 * 1024),
            refusal: None,
        },
        ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "y".repeat(9 * 1024 * 1024),
            refusal: None,
        },
    ];
    assert!(!ModelReplayDelta::new(None, items.clone()).is_valid());

    let replay = ModelReplay::from_checkpoint(
        ModelReplayContract::new("system", Vec::new()),
        items.clone(),
    )
    .expect("a checkpoint root uses the complete replay-prefix budget");
    assert_eq!(replay.items(), items);
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

// replacement binding은 checkpoint seed item을 보존하면서 첫 delta의 새 contract를
// 정확히 한 번 채택해야 하며 ordinary append의 중복-contract 규칙은 그대로 유지합니다.
#[test]
fn replacement_binding_reestablishes_the_replay_contract_once() {
    let seed = ModelReplayItem::Message {
        role: ModelReplayRole::User,
        content: "checkpoint".to_owned(),
        refusal: None,
    };
    let mut replay = ModelReplay::from_checkpoint(
        ModelReplayContract::new("old-system", Vec::new()),
        vec![seed.clone()],
    )
    .unwrap();
    let delta = ModelReplayDelta::new(
        Some(ModelReplayContract::new("new-system", Vec::new())),
        vec![ModelReplayItem::Message {
            role: ModelReplayRole::Assistant,
            content: "continued".to_owned(),
            refusal: None,
        }],
    );

    replay.apply_binding_replacement(&delta).unwrap();
    assert_eq!(
        replay.contract(),
        Some(&ModelReplayContract::new("new-system", Vec::new()))
    );
    assert_eq!(replay.items().first(), Some(&seed));
    assert!(
        replay
            .apply_binding_replacement(&ModelReplayDelta::new(
                None,
                vec![ModelReplayItem::Message {
                    role: ModelReplayRole::Assistant,
                    content: "missing contract".to_owned(),
                    refusal: None,
                }],
            ))
            .unwrap_err()
            .contains("new contract")
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

fn image_replay_part() -> crate::ModelInputPart {
    crate::ModelInputPart::Image { snapshot: serde_json::from_str(r#"{"profile":"yo.input-image-rgba8-triangle/v1","mime_type":"image/png","width":1,"height":1,"byte_length":70,"sha256":"sha256:4ff6ab670a58c14270e034e2090d9a432caa263a14e0a25785386b0c12f880b5","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="}"#).unwrap() }
}

// 반복된 이미지도 독립 occurrence로 replay에 남으며 16개 제한과 empty/adjacent text를 검증한다.
#[test]
fn multimodal_replay_preserves_occurrences_and_validates_user_shape() {
    use crate::ModelInputPart;
    let image = image_replay_part();
    let item = ModelReplayItem::MultimodalUser {
        parts: vec![image.clone(), image.clone()],
    };
    let delta = ModelReplayDelta::new(
        Some(ModelReplayContract::new("system", vec![])),
        vec![item.clone()],
    );
    let mut replay = ModelReplay::default();
    replay.apply(&delta).unwrap();
    assert_eq!(replay.items(), &[item]);
    for parts in [
        vec![],
        vec![ModelInputPart::Text {
            text: "only text".into(),
        }],
        vec![
            ModelInputPart::Text {
                text: String::new(),
            },
            image.clone(),
        ],
        vec![
            ModelInputPart::Text { text: "a".into() },
            ModelInputPart::Text { text: "b".into() },
            image.clone(),
        ],
        vec![image.clone(); 17],
    ] {
        assert!(
            ModelReplayDelta::new(None, vec![ModelReplayItem::MultimodalUser { parts }])
                .validate()
                .is_err()
        );
    }
    assert!(
        ModelReplayDelta::new(
            None,
            vec![ModelReplayItem::MultimodalUser {
                parts: vec![image; 16]
            }]
        )
        .validate()
        .is_ok()
    );
}

// replay 전체 JSON의 16 MiB는 허용하고 escaping을 포함한 첫 초과 byte는 거절한다.
#[test]
fn multimodal_replay_charges_complete_encoded_delta() {
    use crate::ModelInputPart;
    let item = |text: String| ModelReplayItem::MultimodalUser {
        parts: vec![image_replay_part(), ModelInputPart::Text { text }],
    };
    let base = item("x".into());
    let overhead =
        ModelReplayDelta::prospective_encoded_len(None, [&base].into_iter()).unwrap() - 1;
    let text = "x".repeat(MAX_REPLAY_DELTA_BYTES - overhead);
    let exact = item(text.clone());
    assert_eq!(
        ModelReplayDelta::prospective_encoded_len(None, [&exact].into_iter()),
        Some(MAX_REPLAY_DELTA_BYTES)
    );
    ModelReplayDelta::new(None, vec![exact]).validate().unwrap();
    assert!(
        ModelReplayDelta::new(None, vec![item(format!("{text}x"))])
            .validate()
            .is_err()
    );
    assert!(
        ModelReplayDelta::new(None, vec![item(format!("{}\"", &text[..text.len() - 1]))])
            .validate()
            .is_err()
    );
}
