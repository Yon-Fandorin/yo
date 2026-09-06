use super::*;

// enabled omission is the canonical old-readable state. Disabling writes only exact false and
// clears an exact stored model preference in the same CAS; enabling is idempotent and never
// recreates that preference.
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

// The durable activation field is a one-value extension: absence and false are valid, while
// true, null, strings, and other future spellings fail through the ordinary closed decoder.
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

// Reimporting or reconnecting an exact complete binding preserves its operator activation state;
// replacing the complete profile opens a new enabled binding epoch.
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
