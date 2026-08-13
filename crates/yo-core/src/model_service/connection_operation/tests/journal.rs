use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
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
    let debug = format!("{entry:?}");
    assert!(!debug.contains(&private_credential_revision));
    assert!(!debug.contains(&public_revision));
    assert!(!debug.contains("planned_snapshot"));
}

// 새 wire-v1 intent는 legacy profile_digests 칸을 빈 배열로 유지하고, 이전 writer가 남긴
// 올바른 digest는 복구 entry에 영향을 주지 않지만 형식이 틀리면 전체 intent를 거절합니다.
#[test]
fn legacy_profile_digests_are_validated_but_do_not_change_recovery_identity() {
    let fixture = Fixture::new("legacy-profile-digests");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    let current = fs::read_to_string(fixture.journal.path()).unwrap();
    assert!(current.contains("profile_digests: []"));

    let legacy = current.replacen(
        "profile_digests: []",
        &format!("profile_digests:\n- {}", super::support::digest('b')),
        1,
    );
    fs::write(fixture.journal.path(), &legacy).unwrap();
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry));

    fs::write(
        fixture.journal.path(),
        legacy.replacen(&super::support::digest('b'), "sha256:not-a-digest", 1),
    )
    .unwrap();
    assert!(matches!(
        fixture.journal.capture(),
        Err(ConnectionOperationError::InvalidContents(_))
    ));
}

// connect 저널은 intent부터 credential_committed, public_committed, complete 순서로만
// 원자 교체되고 complete의 정확한 현재 entry만 durable remove할 수 있음을 검증합니다.
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
}

// expected/expected 복구가 선택한 uncommitted intent는 complete phase를 위조하지 않고 exact
// intent 그대로 durable remove되어 다음 invocation이 secret 검증부터 새로 시작합니다.
#[test]
fn exact_uncommitted_intent_can_be_abandoned() {
    let fixture = Fixture::new("abandon-intent");
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();

    fixture.journal.abandon_intent(&mut guard, &entry).unwrap();

    assert!(fixture.journal.capture().unwrap().is_none());
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

// revision 없는 legacy credentials.yaml에서 준비한 replace의 metadata receipt도 private
// journal token으로 왕복되어 첫 managed credential commit 전 중단을 정확히 판별합니다.
#[test]
fn legacy_credential_revision_round_trips_as_private_expected_receipt() {
    let fixture = Fixture::new("legacy-revision");
    fs::create_dir_all(fixture.credentials.path().parent().unwrap()).unwrap();
    fs::write(
        fixture.credentials.path(),
        concat!(
            "version: 1\n",
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
    assert!(raw.contains("legacy-"));
    assert!(!raw.contains("retained-secret"));
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry.clone()));
    assert!(!format!("{entry:?}").contains("legacy-"));
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

// journal 경로가 symlink이면 O_NOFOLLOW capture가 target bytes를 읽지 않고 실패해 외부
// permission-restricted 파일을 operation intent로 가져오거나 수정하지 않음을 검증합니다.
#[test]
fn symbolic_journal_path_is_never_followed() {
    let fixture = Fixture::new("symlink");
    fs::create_dir_all(fixture.journal.path().parent().unwrap()).unwrap();
    let target = fixture.journal.path().parent().unwrap().join("target.yaml");
    fs::write(&target, "private-target\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, fixture.journal.path()).unwrap();

    assert!(fixture.journal.capture().is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "private-target\n");
}
