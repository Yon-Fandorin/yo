use super::*;

// last_failure는 complete binding과 분리된 warning-only 상태로 round-trip하고 성공 관찰은
// 값이 있을 때만 새 revision으로 제거하며, 이미 비어 있으면 불필요한 write를 준비하지 않습니다.
#[test]
fn model_observation_round_trips_and_success_clears_only_present_state() {
    let (_directory, repository) = repository("model-observation-round-trip");
    let binding = stored_binding("model-a", "medium");
    let complete = binding.complete().clone();
    let selection = binding.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), binding)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

    let failure =
        ModelLastFailure::new(ModelRequestFailureKind::RateLimited, "2026-08-17T09:10:11Z")
            .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_observation(&selection, &complete, Some(failure.clone()))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

    let captured = repository.capture().unwrap();
    assert_eq!(captured.models()[0].last_failure(), Some(&failure));
    let encoded = fs::read_to_string(repository.path()).unwrap();
    assert!(encoded.contains("kind: rate_limited"));
    assert!(encoded.contains("observed_at: 2026-08-17T09:10:11Z"));

    let mutation = captured
        .prepare_model_observation(&selection, &complete, None)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let cleared = repository.capture().unwrap();
    assert!(cleared.models()[0].last_failure().is_none());
    assert!(
        cleared
            .prepare_model_observation(&selection, &complete, None)
            .unwrap()
            .is_none()
    );
    assert!(
        !fs::read_to_string(repository.path())
            .unwrap()
            .contains("last_failure")
    );
}

// grouped replacement은 같은 좌표와 exact complete binding에만 기존 warning을 보존하고,
// profile이 달라 새 binding epoch가 열리면 오래된 관찰을 새 정의로 넘기지 않습니다.
#[test]
fn grouped_replacement_retains_failure_only_for_an_exact_complete_binding() {
    let (_directory, repository) = repository("group-observation-retention");
    let initial = stored_binding("model-a", "medium");
    let complete = initial.complete().clone();
    let selection = initial.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), initial)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let failure = ModelLastFailure::new(
        ModelRequestFailureKind::Authentication,
        "2026-08-17T09:10:11Z",
    )
    .unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_observation(&selection, &complete, Some(failure.clone()))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();

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
    assert_eq!(
        repository.capture().unwrap().models()[0].last_failure(),
        Some(&failure)
    );

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
    assert!(
        repository.capture().unwrap().models()[0]
            .last_failure()
            .is_none()
    );
}

// durable observation은 closed kind와 canonical UTC whole-second timestamp만 받아들여 null,
// fractional/offset/noncanonical timestamp, unknown kind가 parser-dependent 상태가 되지 않습니다.
#[test]
fn durable_last_failure_shape_is_closed_and_canonical() {
    assert!(
        ModelLastFailure::new(ModelRequestFailureKind::Protocol, "2026-08-17T09:10:11Z").is_ok()
    );
    for invalid in [
        "2026-08-17T09:10:11.1Z",
        "2026-08-17T18:10:11+09:00",
        "2026-08-17 09:10:11Z",
        "not-a-timestamp",
    ] {
        assert!(
            ModelLastFailure::new(ModelRequestFailureKind::Protocol, invalid).is_err(),
            "{invalid}"
        );
    }

    let (_directory, repository) = repository("closed-observation-wire");
    let binding = stored_binding("model-a", "medium");
    let complete = binding.complete().clone();
    let selection = binding.selection();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), binding)
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let failure =
        ModelLastFailure::new(ModelRequestFailureKind::Protocol, "2026-08-17T09:10:11Z").unwrap();
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_observation(&selection, &complete, Some(failure))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let valid = fs::read_to_string(repository.path()).unwrap();

    for (needle, replacement) in [
        ("last_failure:\n", "last_failure: null\n"),
        ("kind: protocol", "kind: future_failure"),
        (
            "observed_at: 2026-08-17T09:10:11Z",
            "observed_at: 2026-08-17T09:10:11.1Z",
        ),
    ] {
        let malformed = valid.replace(needle, replacement);
        assert_ne!(malformed, valid, "fixture must replace {needle:?}");
        fs::write(repository.path(), &malformed).unwrap();
        assert!(matches!(
            repository.capture(),
            Err(ConnectionRepositoryError::InvalidContents(_))
        ));
    }
}
