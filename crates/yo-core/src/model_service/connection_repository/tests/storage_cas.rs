use super::*;

// 파일이 없는 첫 capture는 부모 경로도 만들지 않은 채 absent revision과 unset preference를
// 반환해야 읽기와 준비만 수행한 중단이 사용자 상태를 바꾸지 않음을 검증합니다.
#[test]
fn missing_repository_is_non_creating_canonical_empty_state() {
    let (_directory, repository) = repository("missing");

    let snapshot = repository.capture().unwrap();

    assert_eq!(snapshot.revision(), &ConnectionRevision::Absent);
    assert!(snapshot.preference().is_none());
    assert!(!repository.path().parent().unwrap().exists());
}

// absent 상태에서 준비한 HostTarget CAS는 정확히 한 번 0600 파일로 게시되고 재실행한
// 동일 mutation은 caller가 첫 성공을 못 본 경우에도 AlreadyCommitted로 판정합니다.
#[test]
fn first_publication_is_secure_and_exact_retry_is_idempotent() {
    let (_directory, repository) = repository("first-publication");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_preference(Some(StartupTarget::host_codex()))
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

    let metadata = fs::metadata(repository.path()).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    let current = repository.capture().unwrap();
    assert_eq!(current.revision(), mutation.planned_revision());
    assert_eq!(current.preference(), Some(&StartupTarget::host_codex()));
}

// 연결 저장소는 core에 구체 어댑터 enum을 되살리지 않고 HostId의 canonical reference를
// 저장·복원해 새 Grok 기본값도 Codex와 동일한 CAS 의미를 갖습니다.
#[test]
fn grok_host_preference_round_trips_as_a_generic_host_id() {
    let (_directory, repository) = repository("grok-host");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_preference(Some(StartupTarget::host_grok()))
        .unwrap()
        .unwrap();

    repository.commit(&mutation).unwrap();

    assert_eq!(
        repository.capture().unwrap().preference(),
        Some(&StartupTarget::host_grok())
    );
}

// 같은 expected revision에서 준비한 두 기본값 중 먼저 commit한 승자 뒤에는 loser가
// revision conflict로 실패하고 승자의 preference bytes가 덮어써지지 않음을 검증합니다.
#[test]
fn stale_cas_preserves_the_concurrent_winner() {
    let (_directory, repository) = repository("winner");
    let captured = repository.capture().unwrap();
    let winner = captured
        .prepare_preference(Some(model_target("winner")))
        .unwrap()
        .unwrap();
    let loser = captured
        .prepare_preference(Some(model_target("loser")))
        .unwrap()
        .unwrap();

    repository.commit(&winner).unwrap();
    let error = repository.commit(&loser).unwrap_err();

    assert!(matches!(error, ConnectionRepositoryError::Conflict { .. }));
    assert_eq!(
        repository.capture().unwrap().preference(),
        Some(&model_target("winner"))
    );
}

// 기존 preference를 clear하도록 준비만 하고 CAS 전에 호출이 사라지면 expected 파일은
// 그대로여야 하며, 새 invocation이 같은 clear를 새 revision으로 정상 완료할 수 있습니다.
#[test]
fn interruption_before_cas_leaves_old_state_for_a_fresh_retry() {
    let (_directory, repository) = repository("pre-cas");
    let set = repository
        .capture()
        .unwrap()
        .prepare_preference(Some(StartupTarget::host_codex()))
        .unwrap()
        .unwrap();
    repository.commit(&set).unwrap();
    let before = repository.capture().unwrap();
    let abandoned = before.prepare_preference(None).unwrap().unwrap();
    drop(abandoned);

    assert_eq!(repository.capture().unwrap().revision(), before.revision());
    let retry = repository
        .capture()
        .unwrap()
        .prepare_preference(None)
        .unwrap()
        .unwrap();
    repository.commit(&retry).unwrap();
    assert!(repository.capture().unwrap().preference().is_none());
}

// operation guard가 살아 있는 동안 두 번째 process-equivalent handle은 즉시 Busy를 받고,
// guard 해제 뒤에는 같은 persistent lock file에서 다시 획득되어 장기 hang 없이 직렬화됩니다.
#[test]
fn operation_lock_is_nonblocking_and_reacquirable() {
    let (_directory, repository) = repository("operation-lock");
    let guard = repository.acquire_operation().unwrap();

    assert!(matches!(
        repository.acquire_operation(),
        Err(ConnectionRepositoryError::OperationBusy(_))
    ));
    drop(guard);
    repository.acquire_operation().unwrap();
}

// operation lock 아래에서 pending journal이 보이면 preference-only command가 새 CAS를
// 계획하지 않고 fail-closed해야 아직 구현되지 않은 recoverable operation을 덮지 않습니다.
#[test]
fn pending_operation_blocks_a_new_preference_mutation() {
    let (_directory, repository) = repository("pending-operation");
    let _guard = repository.acquire_operation().unwrap();
    let journal = repository
        .path()
        .parent()
        .unwrap()
        .join(PENDING_OPERATION_FILE);
    fs::write(&journal, "pending\n").unwrap();

    assert!(matches!(
        repository.recover_pending_operation(),
        Err(ConnectionRepositoryError::PendingOperation(found)) if found == journal
    ));
    assert!(!repository.path().exists());
}

// 사용자가 아닌 group에 읽기 권한이 열린 connections.yaml은 내용이 올바르더라도 capture가
// 거절되어 이후 mutation이 안전하지 않은 공개 저장소를 정상 상태로 취급하지 않습니다.
#[test]
fn insecure_existing_snapshot_is_rejected() {
    let (_directory, repository) = repository("permissions");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    fs::write(
        repository.path(),
        "revision: rev-00000000000000000000000000000000\n",
    )
    .unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o640)).unwrap();

    assert!(matches!(
        repository.capture(),
        Err(ConnectionRepositoryError::InsecurePermissions(_))
    ));
}

// 닫힌 stored schema와 맞지 않는 opaque binding/account는 typed state로 오인하지 않고
// capture 단계에서 실패해 기존 상태를 바꾸지 않습니다.
#[test]
fn malformed_models_and_accounts_fail_closed() {
    let (_directory, repository) = repository("unsupported-stored-state");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    fs::write(
        repository.path(),
        concat!(
            "",
            "revision: rev-00000000000000000000000000000000\n",
            "bindings:\n  - binding: retained\n",
            "accounts:\n  - account: retained\n",
        ),
    )
    .unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o600)).unwrap();

    let before = fs::read(repository.path()).unwrap();
    let error = repository.capture().unwrap_err();

    assert!(matches!(
        &error,
        ConnectionRepositoryError::InvalidContents(_)
    ));
    assert_eq!(fs::read(repository.path()).unwrap(), before);
}

// 별도 format classifier가 없으므로 top-level version은 일반 unknown field로 닫히고
// 원문은 자동 migration이나 삭제 없이 보존됩니다.
#[test]
fn unknown_top_level_version_is_invalid_connection_state() {
    let (_directory, repository) = repository("unknown-version");
    fs::create_dir_all(repository.path().parent().unwrap()).unwrap();
    let stale = "version: 1\nrevision: rev-00000000000000000000000000000000\n";
    fs::write(repository.path(), stale).unwrap();
    fs::set_permissions(repository.path(), fs::Permissions::from_mode(0o600)).unwrap();

    let error = repository.capture().unwrap_err();
    assert!(matches!(
        error,
        ConnectionRepositoryError::InvalidContents(_)
    ));
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), stale);
}

// binding 내부의 version 오타도 원문을 보존한 일반 invalid-content 오류로 닫힙니다.
#[test]
fn nested_version_field_is_invalid_connection_state() {
    let (_directory, repository) = repository("nested-version");
    let mutation = repository
        .capture()
        .unwrap()
        .prepare_model_upsert(stored_account(), stored_binding("model-a", "medium"))
        .unwrap()
        .unwrap();
    repository.commit(&mutation).unwrap();
    let original = fs::read_to_string(repository.path()).unwrap();
    let malformed = original.replace("model: model-a", "model: model-a\n    version: 1");
    assert_ne!(malformed, original, "fixture must add the nested field");
    fs::write(repository.path(), &malformed).unwrap();

    let error = repository.capture().unwrap_err();
    assert!(matches!(
        &error,
        ConnectionRepositoryError::InvalidContents(_)
    ));
    assert_eq!(fs::read_to_string(repository.path()).unwrap(), malformed);
}
