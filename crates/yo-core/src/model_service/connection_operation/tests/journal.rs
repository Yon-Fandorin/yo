use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};

use super::{
    super::{
        ConnectionOperationError, ConnectionOperationPhase, error::MAX_OPERATION_JOURNAL_BYTES,
    },
    support::{CANDIDATE_SECRET, Fixture},
};

// 저널이 없는 capture는 경로나 파일을 만들지 않아 preference-only 명령이 단순 확인만으로
// 사용자 상태를 변경하지 않고 안전하게 계속할 수 있음을 검증합니다.
#[test]
fn missing_journal_capture_is_non_creating() {
    let fixture = Fixture::new("missing");

    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(!fixture.journal.path().parent().unwrap().exists());
}

// connect intent는 candidate secret 없이 0600 bounded 파일로 왕복되고 Debug에도 private
// CredentialRevision이나 planned public snapshot bytes가 노출되지 않음을 검증합니다.
#[test]
fn connect_intent_round_trips_without_secret_or_private_debug_receipts() {
    let fixture = Fixture::new("round-trip");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    let private_credential_revision = entry
        .credential_mutation()
        .unwrap()
        .planned_revision()
        .operation_journal_token()
        .to_owned();
    let public_revision = entry.connection_mutation().planned_revision().to_string();

    fixture.journal.publish_intent(&mut guard, &entry).unwrap();

    assert_eq!(fixture.journal.capture().unwrap(), Some(entry.clone()));
    let metadata = fs::metadata(fixture.journal.path()).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    let raw = fs::read_to_string(fixture.journal.path()).unwrap();
    assert!(!raw.contains(CANDIDATE_SECRET));
    assert!(raw.contains(&private_credential_revision));
    assert!(!raw.contains("version:"));
    assert!(!raw.contains("profile_digests:"));
    assert!(!raw.contains("config_snapshot_digest:"));
    let debug = format!("{entry:?}");
    assert!(!debug.contains(&private_credential_revision));
    assert!(!debug.contains(&public_revision));
    assert!(!debug.contains("planned_snapshot"));
}

// version 또는 profile_digests는 current journal의 unknown field이므로 별도 format
// classifier 없이 invalid-content로 거절하고, 실패 뒤에도 입력 bytes를 그대로 보존합니다.
#[test]
fn unknown_journal_fields_are_invalid_contents() {
    let fixture = Fixture::new("unknown-fields");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    let current = fs::read_to_string(fixture.journal.path()).unwrap();

    for unknown_field in [
        "version: 1",
        "profile_digests: []",
        "config_snapshot_digest: sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        let malformed = format!("{unknown_field}\n{current}");
        fs::write(fixture.journal.path(), &malformed).unwrap();
        let error = fixture.journal.capture().unwrap_err();
        assert!(
            matches!(&error, ConnectionOperationError::InvalidContents(_)),
            "{unknown_field}: {error}"
        );
        assert_eq!(
            fs::read(fixture.journal.path()).unwrap(),
            malformed.as_bytes()
        );
    }
}

// connection 내부의 같은 오타도 mutation 전 일반 invalid-content 오류로 거절합니다.
#[test]
fn nested_unknown_names_are_invalid_journal_contents() {
    let fixture = Fixture::new("nested-unknown-names");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    let current = fs::read_to_string(fixture.journal.path()).unwrap();

    for nested_field in ["  version: 1", "  profile_digests: []"] {
        let malformed = current.replacen(
            "connection:\n",
            &format!("connection:\n{nested_field}\n"),
            1,
        );
        assert_ne!(malformed, current, "fixture must add {nested_field}");
        fs::write(fixture.journal.path(), &malformed).unwrap();
        let error = fixture.journal.capture().unwrap_err();
        assert!(
            matches!(&error, ConnectionOperationError::InvalidContents(_)),
            "{nested_field}: {error}"
        );
        assert_eq!(
            fs::read_to_string(fixture.journal.path()).unwrap(),
            malformed
        );
    }
}

// connect 저널은 닫힌 phase 순서로만 교체되며 complete를 clear하면 pathname이 사라지고
// 같은 operation guard로 새 intent를 즉시 게시할 수 있음을 검증합니다.
#[test]
fn connect_phases_advance_in_closed_order_before_exact_clear() {
    let fixture = Fixture::new("phases");
    let mut entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();

    for phase in [
        ConnectionOperationPhase::CredentialCommitted,
        ConnectionOperationPhase::PublicCommitted,
        ConnectionOperationPhase::Complete,
    ] {
        entry = fixture.journal.advance(&mut guard, &entry, phase).unwrap();
        assert_eq!(fixture.journal.capture().unwrap(), Some(entry.clone()));
    }
    fixture.journal.clear_complete(&mut guard, &entry).unwrap();
    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(matches!(
        fs::symlink_metadata(fixture.journal.path()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    ));

    let next = fixture.connect_entry();
    fixture.journal.publish_intent(&mut guard, &next).unwrap();
    assert_eq!(fixture.journal.capture().unwrap(), Some(next));
}

// expected/expected 복구가 선택한 uncommitted intent는 exact remove 후 pathname을 남기지
// 않으며, 같은 operation guard가 새 intent를 즉시 게시할 수 있음을 검증합니다.
#[test]
fn exact_uncommitted_intent_can_be_abandoned() {
    let fixture = Fixture::new("abandon-intent");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();

    fixture.journal.abandon_intent(&mut guard, &entry).unwrap();

    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(matches!(
        fs::symlink_metadata(fixture.journal.path()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    ));

    let next = fixture.connect_entry();
    fixture.journal.publish_intent(&mut guard, &next).unwrap();
    assert_eq!(fixture.journal.capture().unwrap(), Some(next));
}

// 순서를 건너뛴 phase 교체는 게시 전에 거절되고 기존 intent bytes가 그대로 남아 phase가
// 실제 repository state보다 앞서 기록되는 복구 거짓말을 만들지 않음을 검증합니다.
#[test]
fn skipped_phase_is_rejected_without_changing_intent() {
    let fixture = Fixture::new("skip-phase");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    let before = fs::read(fixture.journal.path()).unwrap();

    let error = fixture
        .journal
        .advance(
            &mut guard,
            &entry,
            ConnectionOperationPhase::PublicCommitted,
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ConnectionOperationError::InvalidTransition { .. }
    ));
    assert_eq!(fs::read(fixture.journal.path()).unwrap(), before);
}

// 한 operation-lock owner가 phase를 전진시킨 뒤 stale entry로 다시 교체하려 하면 exact
// entry check가 Conflict를 반환해 새 phase를 과거 내용으로 덮어쓰지 않음을 검증합니다.
#[test]
fn stale_phase_writer_preserves_current_journal() {
    let fixture = Fixture::new("stale-phase");
    let intent = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &intent).unwrap();
    let current = fixture
        .journal
        .advance(
            &mut guard,
            &intent,
            ConnectionOperationPhase::CredentialCommitted,
        )
        .unwrap();

    let error = fixture
        .journal
        .advance(
            &mut guard,
            &intent,
            ConnectionOperationPhase::CredentialCommitted,
        )
        .unwrap_err();

    assert!(matches!(error, ConnectionOperationError::Conflict(_)));
    assert_eq!(fixture.journal.capture().unwrap(), Some(current));
}

// 이미 다른 intent가 durable한 경로에 두 번째 operation을 publish하면 exclusive first
// publication이 Conflict로 실패하고 첫 operation identity와 bytes를 보존함을 검증합니다.
#[test]
fn second_intent_cannot_replace_pending_operation() {
    let fixture = Fixture::new("second-intent");
    let first = fixture.connect_entry();
    let second = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &first).unwrap();

    assert!(matches!(
        fixture.journal.publish_intent(&mut guard, &second),
        Err(ConnectionOperationError::Conflict(_))
    ));
    assert_eq!(fixture.journal.capture().unwrap(), Some(first));
}

// mutable operation guard가 살아 있는 동안 같은 directory의 두 번째 caller는 lock을 얻지
// 못하고, 다른 directory guard도 journal mutation 권한으로 쓸 수 없어 capture와 교체 사이
// interleaving이 API 경계에서 차단됨을 검증합니다.
#[test]
fn journal_mutation_requires_the_exact_exclusive_operation_guard() {
    let fixture = Fixture::new("guard-owner");
    let other = Fixture::new("other-guard");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    let mut wrong_guard = other.operation_guard();

    assert!(matches!(
        fixture.connections.acquire_operation(),
        Err(crate::model_service::ConnectionRepositoryError::OperationBusy(_))
    ));
    assert!(matches!(
        fixture.journal.publish_intent(&mut wrong_guard, &entry),
        Err(ConnectionOperationError::OperationGuardMismatch(_))
    ));
    assert!(fixture.journal.capture().unwrap().is_none());

    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry));
}

// group-readable 저널은 YAML이 올바르더라도 읽기 전에 거절되어 private revision receipt가
// permission-restricted 경계 밖의 파일에서 복구 권한처럼 해석되지 않음을 검증합니다.
#[test]
fn insecure_journal_permissions_fail_closed() {
    let fixture = Fixture::new("permissions");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    fs::set_permissions(fixture.journal.path(), fs::Permissions::from_mode(0o640)).unwrap();

    assert!(matches!(
        fixture.journal.capture(),
        Err(ConnectionOperationError::InsecurePermissions(_))
    ));
}

// 제한보다 큰 regular file은 YAML parser에 전달되지 않고 TooLarge로 실패해 손상되거나
// 공격적인 pending operation이 무제한 메모리 사용을 유발하지 않음을 검증합니다.
#[test]
fn oversized_journal_is_rejected_before_decode() {
    let fixture = Fixture::new("too-large");
    fs::create_dir_all(fixture.journal.path().parent().unwrap()).unwrap();
    let file = fs::File::create(fixture.journal.path()).unwrap();
    file.set_len(MAX_OPERATION_JOURNAL_BYTES + 1).unwrap();
    fs::set_permissions(fixture.journal.path(), fs::Permissions::from_mode(0o600)).unwrap();

    assert!(matches!(
        fixture.journal.capture(),
        Err(ConnectionOperationError::TooLarge(_))
    ));
}

// revision 없는 current-shape credentials.yaml에서 준비한 replace의 derived metadata
// receipt도 private journal token으로 왕복되어 첫 stored credential commit 전 중단을
// 정확히 판별합니다.
#[test]
fn derived_revision_for_revisionless_credentials_round_trips_privately() {
    let fixture = Fixture::new("derived-revision");
    fs::create_dir_all(fixture.credentials.path().parent().unwrap()).unwrap();
    fs::write(
        fixture.credentials.path(),
        concat!(
            "",
            "providers:\n",
            "  qwencloud:\n",
            "    default:\n",
            "      api_key: retained-secret\n",
        ),
    )
    .unwrap();
    fs::set_permissions(
        fixture.credentials.path(),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();

    fixture.journal.publish_intent(&mut guard, &entry).unwrap();

    let raw = fs::read_to_string(fixture.journal.path()).unwrap();
    assert!(raw.contains("derived-"));
    assert!(!raw.contains("retained-secret"));
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry.clone()));
    assert!(!format!("{entry:?}").contains("derived-"));
}

// wire kind를 disconnect로 바꾸고 add action은 그대로 둔 closed-schema 위조는 조합 검증에서
// InvalidContents로 실패해 recovery가 계약에 없는 operation/action 상태표를 선택하지 않습니다.
#[test]
fn invalid_operation_and_credential_action_combination_is_rejected() {
    let fixture = Fixture::new("invalid-combination");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    let raw = fs::read_to_string(fixture.journal.path()).unwrap();
    let invalid = raw.replacen("kind: connect_credential_change", "kind: disconnect", 1);
    assert_ne!(invalid, raw);
    fs::write(fixture.journal.path(), invalid).unwrap();

    assert!(matches!(
        fixture.journal.capture(),
        Err(ConnectionOperationError::InvalidContents(_))
    ));
}

// journal 경로가 유효한 다른 intent를 가리키는 symlink여도 capture가 그 entry를 가져오지
// 않고 실패하며, 외부 target의 inode와 bytes를 수정하지 않음을 검증합니다.
#[test]
fn symbolic_journal_path_is_never_followed() {
    let fixture = Fixture::new("symlink");
    let foreign = Fixture::new("symlink-foreign");
    let foreign_entry = foreign.connect_entry();
    let mut foreign_guard = foreign.operation_guard();
    foreign
        .journal
        .publish_intent(&mut foreign_guard, &foreign_entry)
        .unwrap();
    let target = foreign.journal.path();
    let target_before = fs::read(target).unwrap();
    let metadata_before = fs::metadata(target).unwrap();
    fs::create_dir_all(fixture.journal.path().parent().unwrap()).unwrap();
    symlink(target, fixture.journal.path()).unwrap();

    assert!(matches!(
        fixture.journal.capture(),
        Err(ConnectionOperationError::Io { ref path, .. }) if path == fixture.journal.path()
    ));
    assert_eq!(fs::read(target).unwrap(), target_before);
    let metadata_after = fs::metadata(target).unwrap();
    assert_eq!(metadata_after.dev(), metadata_before.dev());
    assert_eq!(metadata_after.ino(), metadata_before.ino());
    assert_eq!(foreign.journal.capture().unwrap(), Some(foreign_entry));
}
