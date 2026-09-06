use super::*;

// 첫 stored upsert는 account와 complete binding을 readable typed YAML로 함께 게시하고,
// 첫 성공 preference를 같은 CAS에 포함하며 exact retry 뒤에도 typed 값이 보존됩니다.
#[test]
fn stored_upsert_round_trips_complete_state_and_first_preference() {
    let (_directory, repository) = repository("stored-upsert");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();

    assert_eq!(
        repository.commit(&mutation).unwrap(),
        ConnectionCommit::Committed
    );
    assert_eq!(
        repository.commit(&mutation).unwrap(),
        ConnectionCommit::AlreadyCommitted
    );
    let captured = repository.capture().unwrap();

    assert_eq!(captured.accounts(), &[stored_account()]);
    assert_eq!(captured.models(), &[stored_binding("model-a", "medium")]);
    assert_eq!(captured.preference(), Some(&model_target("model-a")));
    let encoded = fs::read_to_string(repository.path()).unwrap();
    assert!(!encoded.contains("version:"));
    assert!(encoded.contains("reasoning_parameters:"));
    assert!(encoded.contains("effort: medium"));
    assert!(!encoded.contains("verification_profile"));
    assert!(!encoded.contains("secret"));
}

// unknown output binding을 저장하면 connections.yaml producer가 max_output_tokens를
// 생략하고 capture가 None을 그대로 복원해 0이나 임의의 숫자로 바꾸지 않습니다.
#[test]
fn stored_unknown_output_limit_round_trips_by_omission() {
    let (_directory, repository) = repository("unknown-output");
    let binding = stored_binding_with_unknown_output("model-a");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), binding.clone())
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

    let encoded = fs::read_to_string(repository.path()).unwrap();
    assert!(!encoded.contains("max_output_tokens"));
    assert_eq!(repository.capture().unwrap().models(), &[binding]);
}

// connections.yaml의 profile은 실행 동작만 저장하므로 폐기된 연결 검증 필드를
// unknown field로 닫고 원문을 자동 변환하지 않습니다.
#[test]
fn durable_profile_rejects_connection_verification_field() {
    let (_directory, repository) = repository("verification-field");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let original = fs::read_to_string(repository.path()).unwrap();
    let malformed = original.replace(
        "tool_capability_policy: local-tools/v1",
        "tool_capability_policy: local-tools/v1\n      verification_profile: semantic-terminal/v1",
    );
    assert_ne!(malformed, original, "fixture must add the retired field");
    fs::write(repository.path(), &malformed).unwrap();

    assert!(matches!(
        repository.capture(),
        Err(ConnectionRepositoryError::InvalidContents(_))
    ));
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), malformed);
}

// durable structured profile 전체의 null은 recursive 내부 null과 구분되어 capture에서
// 실패하고 기존 connections.yaml bytes를 수정하지 않습니다.
#[test]
fn durable_profile_whole_field_null_is_rejected() {
    let (_directory, repository) = repository("whole-field-null");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let encoded = fs::read_to_string(repository.path()).unwrap();
    let malformed = encoded.replace(
        "reasoning_parameters:\n      effort: medium",
        "reasoning_parameters: null",
    );
    assert_ne!(
        malformed, encoded,
        "fixture must replace the structured field"
    );
    fs::write(repository.path(), &malformed).unwrap();

    assert!(matches!(
        repository.capture(),
        Err(ConnectionRepositoryError::InvalidContents(_))
    ));
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), malformed);
}

// Optional durable fields distinguish omission from an explicitly authored null. Null is never
// accepted as an alias for "not present" at the connections.yaml boundary.
#[test]
fn durable_optional_whole_field_nulls_are_rejected() {
    let replacements = [
        (
            "provider_display_name: QwenCloud",
            "provider_display_name: null",
        ),
        (
            "account_display_name: Default",
            "account_display_name: null",
        ),
        (
            "model_display_name: Model model-a",
            "model_display_name: null",
        ),
        (
            "preference:\n  kind: model\n  provider: qwencloud\n  account: default\n  model: model-a",
            "preference: null",
        ),
        ("max_output_tokens: 100", "max_output_tokens: null"),
    ];
    for (index, (needle, replacement)) in replacements.into_iter().enumerate() {
        let (_directory, repository) = repository(&format!("optional-null-{index}"));
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        let encoded = fs::read_to_string(repository.path()).unwrap();
        let malformed = encoded.replace(needle, replacement);
        assert_ne!(malformed, encoded, "fixture must replace {needle}");
        fs::write(repository.path(), &malformed).unwrap();

        assert!(matches!(
            repository.capture(),
            Err(ConnectionRepositoryError::InvalidContents(_))
        ));
        assert_eq!(fs::read_to_string(repository.path()).unwrap(), malformed);
    }
}

// connections.yaml reader는 명시적 semantic-only도 호환 입력으로 읽지만 producer는 계속
// 생략하고, null·unknown·duplicate는 private profile이나 omission으로 축약하지 않습니다.
#[test]
fn stored_replay_profile_is_presence_aware_and_closed() {
    let (_directory, semantic_repository) = repository("stored-explicit-semantic-replay-profile");
    let mutation = semantic_repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_kimi_account(), stored_kimi_binding())
        .unwrap()
        .unwrap();
    semantic_repository.commit(&mutation).unwrap();
    let private = fs::read_to_string(semantic_repository.path()).unwrap();
    fs::write(
        semantic_repository.path(),
        private.replace(
            "replay_profile: kimi-private-local-plaintext/v1",
            "replay_profile: semantic-only/v1",
        ),
    )
    .unwrap();
    let explicit_semantic = semantic_repository.capture().unwrap();
    assert_eq!(
        explicit_semantic.models()[0]
            .complete()
            .profile()
            .replay_profile()
            .as_str(),
        "semantic-only/v1"
    );

    for replacement in [
        "replay_profile: null",
        "replay_profile: unknown/v1",
        concat!(
            "replay_profile: kimi-private-local-plaintext/v1\n",
            "      replay_profile: kimi-private-local-plaintext/v1"
        ),
    ] {
        let (_directory, repository) = repository("stored-replay-profile");
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(stored_kimi_account(), stored_kimi_binding())
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
        let original = fs::read_to_string(repository.path()).unwrap();
        let malformed = original.replace(
            "replay_profile: kimi-private-local-plaintext/v1",
            replacement,
        );
        fs::write(repository.path(), malformed).unwrap();

        assert!(
            matches!(
                repository.capture(),
                Err(ConnectionRepositoryError::InvalidContents(_))
            ),
            "{replacement}"
        );
    }
}

// connections.yaml은 Kimi private replay 동의를 readable profile field로 보존하지만
// 비밀이나 provider-private Session payload 자체는 섞지 않고 capture에서 같은 타입을 복원합니다.
#[test]
fn stored_kimi_binding_round_trips_the_private_replay_authorization_only() {
    let (_directory, repository) = repository("stored-kimi-private-replay");
    let account = ConnectionAccount::new(
        ProviderId::new("kimi").unwrap(),
        AccountId::new("team").unwrap(),
        Some("Kimi".to_owned()),
        Some("Team".to_owned()),
    )
    .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(account, stored_kimi_binding())
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

    let encoded = fs::read_to_string(repository.path()).unwrap();
    assert!(encoded.contains("replay_profile: kimi-private-local-plaintext/v1"));
    assert!(!encoded.contains("reasoning_content"));
    assert!(!encoded.contains("candidate"));
    assert_eq!(
        repository.capture().unwrap().models(),
        &[stored_kimi_binding()]
    );
}
// Discovery의 기존 wire kind는 생성 시 다른 Provider를 거절하고 저장·재읽기에도 같은 제약을
// 유지합니다.
#[test]
fn discovery_seed_rejects_mismatched_wire_identity_before_mutation() {
    let binding = stored_binding("model", "low");
    let complete = binding.complete();
    let (directory, repository) = repository("discovery-identity");
    let seed_for = |provider| {
        ConnectionCatalogSeed::discovery(
            ProviderId::new(provider).unwrap(),
            AccountId::new("default").unwrap(),
            None,
            None,
            complete.binding().endpoint().clone(),
            complete.profile().clone(),
        )
    };
    assert_eq!(
        seed_for("other").unwrap_err().to_string(),
        "OpenRouter discovery seed requires ProviderId openrouter"
    );
    assert!(!repository.path().exists());
    let seed = seed_for("openrouter").unwrap();
    let account =
        ConnectionAccount::new(seed.provider().clone(), seed.account().clone(), None, None)
            .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_group_replace(account, Vec::new(), Some(seed.clone()))
        .unwrap();
    repository.commit(&mutation).unwrap();
    assert_eq!(repository.capture().unwrap().catalog_seeds(), &[seed]);
    let invalid = fs::read_to_string(repository.path())
        .unwrap()
        .replace("provider: openrouter", "provider: other");
    fs::write(repository.path(), invalid).unwrap();
    assert!(repository.capture().is_err());
    drop(directory);
}
