use super::*;

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
