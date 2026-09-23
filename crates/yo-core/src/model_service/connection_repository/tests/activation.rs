use super::*;

// enabled 생략은 이전 형식과 호환되는 기본 상태다. 비활성화는 정확한 false만 기록하고
// 같은 CAS에서 정확히 일치하는 저장 모델 선호도만 지운다. 활성화는 멱등적이며 선호도를 복원하지
// 않는다.
#[test]
fn model_activation_round_trips_and_clears_only_the_disabled_default() {
    let (_directory, repository) = repository("model-activation-round-trip");
    let binding = stored_binding("model-a", "medium");
    let selection = binding.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), binding)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    assert!(
        !fs::read_to_string(repository.path())
            .unwrap()
            .contains("enabled:")
    );

    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_activation(&selection, false)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let disabled = repository.capture().unwrap();
    assert!(!disabled.models()[0].is_enabled());
    assert!(disabled.preference().is_none());
    assert!(
        fs::read_to_string(repository.path())
            .unwrap()
            .contains("enabled: false")
    );
    assert!(
        disabled
            .prepare_model_activation(&selection, false)
            .unwrap()
            .is_none()
    );

    let mutation = disabled
        .prepare_model_activation(&selection, true)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let enabled = repository.capture().unwrap();
    assert!(enabled.models()[0].is_enabled());
    assert!(enabled.preference().is_none());
    assert!(
        !fs::read_to_string(repository.path())
            .unwrap()
            .contains("enabled:")
    );
    assert!(
        enabled
            .prepare_model_activation(&selection, true)
            .unwrap()
            .is_none()
    );
}

// 영속 활성화 필드는 값 하나만 추가하는 확장이다. 생략 또는 false만 유효하고 true, null,
// 문자열과 이후에 추가될 다른 표기는 닫힌 디코더에서 거부된다.
#[test]
fn durable_activation_accepts_only_exact_false_when_present() {
    let (_directory, repository) = repository("closed-activation-wire");
    let binding = stored_binding("model-a", "medium");
    let selection = binding.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), binding)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_activation(&selection, false)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let valid = fs::read_to_string(repository.path()).unwrap();

    for replacement in ["enabled: true", "enabled: null", "enabled: disabled"] {
        let malformed = valid.replace("enabled: false", replacement);
        fs::write(repository.path(), malformed).unwrap();
        assert!(matches!(
            repository.capture(),
            Err(ConnectionRepositoryError::InvalidContents(_))
        ));
    }
}

// 완전히 동일한 바인딩을 다시 가져오거나 연결하면 운영자가 정한 활성 상태를 유지한다.
// 전체 프로필이 달라지면 새 활성 바인딩 epoch가 시작된다.
#[test]
fn exact_binding_republication_preserves_activation_but_changed_binding_enables() {
    let (_directory, repository) = repository("activation-republication");
    let initial = stored_binding("model-a", "medium");
    let selection = initial.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), initial)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_activation(&selection, false)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_connect(stored_account(), stored_binding("model-a", "medium"))
        .unwrap();
    repository.commit(&mutation).unwrap();
    let reconnected = repository.capture().unwrap();
    assert!(!reconnected.models()[0].is_enabled());
    assert!(reconnected.preference().is_none());

    let mutation = repository
        .capture()
        .unwrap()
        .prepare_group_replace(
            stored_account(),
            vec![stored_binding("model-a", "medium")],
            None,
        )
        .unwrap();
    repository.commit(&mutation).unwrap();
    let reimported = repository.capture().unwrap();
    assert!(!reimported.models()[0].is_enabled());
    assert!(reimported.preference().is_none());

    let mutation = repository
        .capture()
        .unwrap()
        .prepare_group_replace(
            stored_account(),
            vec![stored_binding("model-a", "high")],
            None,
        )
        .unwrap();
    repository.commit(&mutation).unwrap();
    assert!(repository.capture().unwrap().models()[0].is_enabled());
}
