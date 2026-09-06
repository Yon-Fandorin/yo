use super::*;

// credential rotation은 public binding이 byte-for-byte 같아도 새 planned revision을 만들어
// journal recovery가 credential-first 절단점 뒤 exact public winner를 식별할 수 있습니다.
#[test]
fn stored_connect_forces_a_new_public_revision_for_equal_state() {
    let (_directory, repository) = repository("stored-connect-epoch");
    let first = repository
        .capture()
        .unwrap()
        .prepare_model_connect(stored_account(), stored_binding("model-a", "medium"))
        .unwrap();
    repository.commit(&first).unwrap();
    let captured = repository.capture().unwrap();

    let rotation = captured
        .prepare_model_connect(stored_account(), stored_binding("model-a", "medium"))
        .unwrap();

    assert_ne!(rotation.expected_revision(), rotation.planned_revision());
    assert_eq!(rotation.expected_revision(), captured.revision());
    repository.commit(&rotation).unwrap();
    assert_eq!(repository.capture().unwrap().models(), captured.models());
}

// Catalog seed도 model definition과 같은 repository revision에 저장되며, producer는 account
// 표시 이름을 catalog row에 중복 쓰지 않고 durable account 한 곳에서만 복원합니다.
#[test]
fn catalog_only_group_round_trips_as_one_stored_definition() {
    let (_directory, repository) = repository("catalog-group");
    let account = stored_account();
    let seed = ConnectionCatalogSeed::built_in(
        VersionedProfileId::new("qwencloud-token-plan-team-intl/v1").unwrap(),
        account.provider_id().clone(),
        account.account_id().clone(),
        account.provider_display_name().map(str::to_owned),
        account.account_display_name().map(str::to_owned),
    )
    .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_group_replace(account, Vec::new(), Some(seed.clone()))
        .unwrap();
    repository.commit(&mutation).unwrap();

    let snapshot = repository.capture().unwrap();
    assert_eq!(snapshot.catalog_seeds(), &[seed]);
    assert!(snapshot.models().is_empty());
    assert!(snapshot.preference().is_none());
    assert_eq!(
        snapshot
            .catalog_seed(
                &ProviderId::new("qwencloud").unwrap(),
                &AccountId::new("default").unwrap(),
            )
            .unwrap()
            .built_in_profile()
            .unwrap()
            .as_str(),
        "qwencloud-token-plan-team-intl/v1"
    );
    let raw = fs::read_to_string(repository.path()).unwrap();
    assert!(raw.contains("kind: built_in"));
    assert_eq!(raw.matches("provider_display_name:").count(), 1);
    assert_eq!(raw.matches("account_display_name:").count(), 1);
}

// Group replacement은 같은 pair의 목록을 merge하지 않고 통째로 바꾸며, 제거된 exact
// ModelTarget preference도 같은 public CAS에서 함께 지웁니다.
#[test]
fn group_replace_removes_omitted_models_and_their_preference() {
    let (_directory, repository) = repository("whole-group-replace");
    let first = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&first).unwrap();
    let replacement = repository
        .capture()
        .unwrap()
        .prepare_group_replace(
            stored_account(),
            vec![stored_binding("model-b", "medium")],
            None,
        )
        .unwrap();
    repository.commit(&replacement).unwrap();

    let snapshot = repository.capture().unwrap();
    assert!(snapshot.preference().is_none());
    assert_eq!(snapshot.models().len(), 1);
    assert_eq!(snapshot.models()[0].selection().model().as_str(), "model-b");
}

// 같은 account에 model을 추가하거나 기존 model profile을 교체할 때 unrelated binding과
// 이미 정해진 preference는 그대로 남고 exact coordinate 하나만 바뀝니다.
#[test]
fn stored_upsert_preserves_unrelated_binding_and_existing_preference() {
    let (_directory, repository) = repository("stored-replace");
    for binding in [
        stored_binding("model-a", "medium"),
        stored_binding("model-b", "medium"),
    ] {
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(stored_account(), binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
    }
    let replacement = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-b", "high"))
        .unwrap()
        .unwrap();
    repository.commit(&replacement).unwrap();

    let captured = repository.capture().unwrap();
    assert_eq!(
        captured.models(),
        &[
            stored_binding("model-a", "medium"),
            stored_binding("model-b", "high"),
        ]
    );
    assert_eq!(captured.preference(), Some(&model_target("model-a")));
}

// stored remove는 선택한 binding과 exact matching preference를 함께 지우되 같은 account의
// 다른 model이 있으면 account를 보존하고, 마지막 binding을 지운 뒤에만 account를 제거합니다.
#[test]
fn stored_remove_preserves_shared_account_then_clears_last_account_and_preference() {
    let (_directory, repository) = repository("stored-remove");
    for binding in [
        stored_binding("model-a", "medium"),
        stored_binding("model-b", "medium"),
    ] {
        let mutation = repository
            .capture()
            .unwrap()
            .prepare_model_upsert(stored_account(), binding)
            .unwrap()
            .unwrap();
        repository.commit(&mutation).unwrap();
    }

    let remove_a = repository
        .capture()
        .unwrap()
        .prepare_model_remove(match model_target("model-a") {
            StartupTarget::Model(ref selection) => selection,
            StartupTarget::Host(_) => unreachable!(),
        })
        .unwrap();
    repository.commit(&remove_a).unwrap();
    let after_a = repository.capture().unwrap();
    assert!(after_a.preference().is_none());
    assert_eq!(after_a.accounts(), &[stored_account()]);
    assert_eq!(after_a.models(), &[stored_binding("model-b", "medium")]);

    let remove_b = after_a
        .prepare_model_remove(match model_target("model-b") {
            StartupTarget::Model(ref selection) => selection,
            StartupTarget::Host(_) => unreachable!(),
        })
        .unwrap();
    repository.commit(&remove_b).unwrap();
    let empty = repository.capture().unwrap();
    assert!(empty.accounts().is_empty());
    assert!(empty.models().is_empty());
}

// preference-only CAS는 typed stored arrays를 다시 encode할 때도 의미를 바꾸거나 버리지
// 않아 default 변경이 connection catalog를 손상하지 않습니다.
#[test]
fn preference_mutation_preserves_stored_state() {
    let (_directory, repository) = repository("preference-preserves-stored");
    let stored = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&stored).unwrap();
    let before = repository.capture().unwrap();
    let preference = before
        .prepare_preference(Some(StartupTarget::host_codex()))
        .unwrap()
        .unwrap();
    repository.commit(&preference).unwrap();

    let after = repository.capture().unwrap();
    assert_eq!(after.accounts(), before.accounts());
    assert_eq!(after.models(), before.models());
    assert_eq!(after.preference(), Some(&StartupTarget::host_codex()));
}

// 새 API는 reserved host Provider를 만들 수 없지만 이미 존재하는 durable host coordinate는
// decoder가 읽어 보존해 이전 공개 상태를 새 binary가 임의로 소실하지 않습니다.
#[test]
fn new_host_stored_state_is_forbidden_while_existing_durable_state_remains_readable() {
    assert!(
        ConnectionAccount::new(
            ProviderId::new("host").unwrap(),
            AccountId::new("default").unwrap(),
            None,
            None,
        )
        .is_err()
    );

    let (_directory, repository) = repository("legacy-host");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let encoded = fs::read_to_string(repository.path())
        .unwrap()
        .replace("qwencloud", "host");
    fs::write(repository.path(), encoded).unwrap();

    let captured = repository.capture().unwrap();
    assert_eq!(captured.accounts()[0].provider_id().as_str(), "host");
    assert_eq!(
        captured.models()[0]
            .complete()
            .binding()
            .provider_id()
            .as_str(),
        "host"
    );
}

// stored YAML도 64-bit 경계의 exact integer/float/String variant를 보존하고, non-finite
// overflow는 거절하며 사용자가 명시적으로 quote한 같은 spelling만 String으로 보존합니다.
#[test]
fn stored_profile_numbers_pin_range_fallback_and_reject_nonfinite_values() {
    let (_directory, repository) = repository("stored-profile-number");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let original = fs::read_to_string(repository.path()).unwrap();

    for (authored, expected) in [
        ("18446744073709551615", serde_json::json!(u64::MAX)),
        (
            "18446744073709551616",
            serde_json::json!(18_446_744_073_709_551_616.0_f64),
        ),
        (
            "340282366920938463463374607431768211456",
            serde_json::json!(340_282_366_920_938_463_463_374_607_431_768_211_456.0_f64),
        ),
        ("-9223372036854775808", serde_json::json!(i64::MIN)),
        (
            "-9223372036854775809",
            serde_json::json!(-9_223_372_036_854_775_809.0_f64),
        ),
        ("0xffffffffffffffff", serde_json::json!(u64::MAX)),
        (
            "0x10000000000000000",
            serde_json::json!("0x10000000000000000"),
        ),
    ] {
        let replacement = original.replace("effort: medium", &format!("value: {authored}"));
        fs::write(repository.path(), replacement).unwrap();
        let captured = repository
            .capture()
            .unwrap_or_else(|error| panic!("{authored}: {error}"));
        assert_eq!(
            captured.models()[0]
                .complete()
                .profile()
                .reasoning_parameters()
                .to_json_value()["value"],
            expected,
            "{authored}"
        );
    }

    let overflow = original.replace("effort: medium", "value: 1e400");
    fs::write(repository.path(), &overflow).unwrap();
    assert!(matches!(
        repository.capture(),
        Err(ConnectionRepositoryError::InvalidContents(_))
    ));

    let quoted = original.replace("effort: medium", "value: '1e400'");
    fs::write(repository.path(), quoted).unwrap();
    let captured = repository.capture().unwrap();
    assert_eq!(
        captured.models()[0]
            .complete()
            .profile()
            .reasoning_parameters()
            .to_json_value(),
        serde_json::json!({"value": "1e400"})
    );
}
