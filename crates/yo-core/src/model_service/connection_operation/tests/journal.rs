use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, PermissionsExt, symlink},
    },
    path::{Path, PathBuf},
};

use super::{
    super::{
        ConnectionOperationError, ConnectionOperationExecutionError,
        ConnectionOperationExecutionOutcome, ConnectionOperationPhase,
        LocalConnectionOperationRepositories, error::MAX_OPERATION_JOURNAL_BYTES, storage,
    },
    support::{CANDIDATE_SECRET, Fixture},
};

fn pending_residue(parent: &Path, hex: &str) -> PathBuf {
    assert_eq!(hex.len(), 32);
    parent.join(format!(".connection-operation.{hex}.pending"))
}

fn write_mode(path: &Path, bytes: &[u8], mode: u32) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn metadata_identity(path: &Path) -> (u64, u64, u32, u64) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.len(),
    )
}

// 저널이 없는 capture는 경로나 파일을 만들지 않아 preference-only 명령이 단순 확인만으로
// 사용자 상태를 변경하지 않고 안전하게 계속할 수 있음을 검증합니다.
#[test]
fn missing_journal_capture_is_non_creating() {
    let fixture = Fixture::new("missing");

    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(!fixture.journal.path().parent().unwrap().exists());
}

// crash가 exact residue만 남기고 canonical journal을 게시하지 못한 다음 startup recovery는
// retained operation guard 아래 residue를 지우고 public/private 상태 없이 NoPending을 냅니다.
#[test]
fn recovery_cleans_exact_residue_when_canonical_journal_is_absent() {
    let fixture = Fixture::new("recovery-cleans-residue-without-canonical");
    let parent = fixture.journal.path().parent().unwrap();
    let guard = fixture.operation_guard();
    let residue = pending_residue(parent, "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
    write_mode(&residue, b"crash-left partial journal", 0o600);
    drop(guard);
    let repositories = LocalConnectionOperationRepositories::in_directory(parent).unwrap();
    let mut session = repositories.acquire().unwrap();

    assert_eq!(
        session.recover_pending_operation().unwrap(),
        ConnectionOperationExecutionOutcome::NoPendingOperation
    );
    assert!(!residue.exists());
    assert!(fixture.journal.capture().unwrap().is_none());
    assert!(
        fixture
            .connections
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
    assert!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
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
    let residue = pending_residue(
        fixture.journal.path().parent().unwrap(),
        "0123456789abcdef0123456789abcdef",
    );
    write_mode(&residue, b"partial", 0o600);

    assert!(matches!(
        fixture.connections.acquire_operation(),
        Err(crate::model_service::ConnectionRepositoryError::OperationBusy(_))
    ));
    assert!(matches!(
        fixture.journal.publish_intent(&mut wrong_guard, &entry),
        Err(ConnectionOperationError::OperationGuardMismatch(_))
    ));
    assert!(fixture.journal.capture().unwrap().is_none());
    assert_eq!(fs::read(&residue).unwrap(), b"partial");

    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    assert!(!residue.exists());
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry));
}

// 서로 다른 empty·partial·최대크기 exact residue를 첫 게시와 phase 교체 전에 모두
// 제거하고, 이후 새 intent의 기존 recovery가 expected/expected abandonment로 수렴합니다.
#[test]
fn eligible_pending_residues_are_cleaned_before_publish_advance_and_recovery() {
    let fixture = Fixture::new("eligible-pending-residues");
    let parent = fixture.journal.path().parent().unwrap();
    let mut guard = fixture.operation_guard();
    let empty = pending_residue(parent, "00000000000000000000000000000000");
    let partial = pending_residue(parent, "11111111111111111111111111111111");
    let maximum = pending_residue(parent, "22222222222222222222222222222222");
    write_mode(&empty, b"", 0o600);
    write_mode(&partial, b"not a complete journal", 0o600);
    let maximum_file = fs::File::create(&maximum).unwrap();
    maximum_file.set_len(MAX_OPERATION_JOURNAL_BYTES).unwrap();
    fs::set_permissions(&maximum, fs::Permissions::from_mode(0o600)).unwrap();

    let mut entry = fixture.connect_entry();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    assert!(!empty.exists());
    assert!(!partial.exists());
    assert!(!maximum.exists());

    let replacement_residue = pending_residue(parent, "33333333333333333333333333333333");
    write_mode(&replacement_residue, b"interrupted replacement", 0o600);
    entry = fixture
        .journal
        .advance(
            &mut guard,
            &entry,
            ConnectionOperationPhase::CredentialCommitted,
        )
        .unwrap();
    assert!(!replacement_residue.exists());
    entry = fixture
        .journal
        .advance(
            &mut guard,
            &entry,
            ConnectionOperationPhase::PublicCommitted,
        )
        .unwrap();
    entry = fixture
        .journal
        .advance(&mut guard, &entry, ConnectionOperationPhase::Complete)
        .unwrap();
    fixture.journal.clear_complete(&mut guard, &entry).unwrap();

    let recoverable = fixture.connect_entry();
    fixture
        .journal
        .publish_intent(&mut guard, &recoverable)
        .unwrap();
    drop(guard);
    let repositories = LocalConnectionOperationRepositories::in_directory(parent).unwrap();
    let mut session = repositories.acquire().unwrap();
    assert!(matches!(
        session.recover_pending_operation().unwrap(),
        ConnectionOperationExecutionOutcome::Abandoned { .. }
    ));
    assert!(fixture.journal.capture().unwrap().is_none());
}

// 결정적으로 safe→wrong-mode 순서로 검증해도 delete callback은 호출되지 않아 첫 pass가
// 모든 exact residue를 승인하기 전에는 어느 residue나 canonical journal도 지우지 않습니다.
#[test]
fn unsafe_pending_residue_blocks_all_cleanup_before_canonical_publication() {
    let fixture = Fixture::new("two-pass-residue-validation");
    let parent = fixture.journal.path().parent().unwrap();
    let mut guard = fixture.operation_guard();
    let eligible = pending_residue(parent, "00000000000000000000000000000000");
    let unsafe_mode = pending_residue(parent, "ffffffffffffffffffffffffffffffff");
    write_mode(&eligible, b"eligible", 0o600);
    write_mode(&unsafe_mode, b"unsafe", 0o400);
    let entry = fixture.connect_entry();
    let remove_called = Cell::new(false);

    assert!(matches!(
        storage::cleanup_pending_residues_in_order_for_test(
            parent,
            &[eligible.clone(), unsafe_mode.clone()],
            &[],
            |path| {
                remove_called.set(true);
                fs::remove_file(path)
            },
            |directory| directory.sync_all(),
        ),
        Err(ConnectionOperationError::InsecurePermissions(ref path)) if path == &unsafe_mode
    ));
    assert!(!remove_called.get());
    assert_eq!(fs::read(&eligible).unwrap(), b"eligible");
    assert_eq!(fs::read(&unsafe_mode).unwrap(), b"unsafe");
    assert!(fixture.journal.capture().unwrap().is_none());

    fs::set_permissions(&unsafe_mode, fs::Permissions::from_mode(0o600)).unwrap();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    assert!(!eligible.exists());
    assert!(!unsafe_mode.exists());
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry));
}

// exact-name symlink 때문에 startup recovery가 실패할 때 symlink와 외부 target, canonical
// intent와 public/private 상태가 모두 그대로 남아 no-follow와 fail-closed를 증명합니다.
#[test]
fn exact_pending_symlink_blocks_recovery_without_following_or_mutating() {
    let fixture = Fixture::new("pending-symlink");
    let parent = fixture.journal.path().parent().unwrap();
    let mut guard = fixture.operation_guard();
    let entry = fixture.connect_entry();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    let canonical_before = fs::read(fixture.journal.path()).unwrap();
    let canonical_identity = metadata_identity(fixture.journal.path());
    let target = parent.join("unrelated-target");
    write_mode(&target, b"foreign bytes", 0o600);
    let target_identity = metadata_identity(&target);
    let residue = pending_residue(parent, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    symlink(&target, &residue).unwrap();
    drop(guard);
    let repositories = LocalConnectionOperationRepositories::in_directory(parent).unwrap();
    let mut session = repositories.acquire().unwrap();

    assert!(matches!(
        session.recover_pending_operation(),
        Err(ConnectionOperationExecutionError::JournalCapture(
            ConnectionOperationError::UnsupportedFileType(ref path)
        )) if path == &residue
    ));
    assert!(
        fs::symlink_metadata(&residue)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&target).unwrap(), b"foreign bytes");
    assert_eq!(metadata_identity(&target), target_identity);
    assert_eq!(fs::read(fixture.journal.path()).unwrap(), canonical_before);
    assert_eq!(
        metadata_identity(fixture.journal.path()),
        canonical_identity
    );
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry));
    assert!(
        fixture
            .connections
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
    assert!(
        fixture
            .credentials
            .capture()
            .unwrap()
            .revision()
            .is_absent()
    );
}

// exact residue 검증 뒤 주입한 unlink PermissionDenied는 residue와 canonical intent의
// inode·bytes를 모두 보존하여 권한이나 실행 user에 의존하지 않는 fail-closed를 증명합니다.
#[test]
fn pending_residue_unlink_failure_preserves_residue_and_canonical_journal() {
    let fixture = Fixture::new("pending-unlink-failure");
    let parent = fixture.journal.path().parent().unwrap();
    let mut guard = fixture.operation_guard();
    let entry = fixture.connect_entry();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    let canonical_before = fs::read(fixture.journal.path()).unwrap();
    let canonical_identity = metadata_identity(fixture.journal.path());
    let residue = pending_residue(parent, "dddddddddddddddddddddddddddddddd");
    write_mode(&residue, b"eligible but not unlinkable", 0o600);
    let residue_identity = metadata_identity(&residue);
    let result = storage::cleanup_pending_residues_in_order_for_test(
        parent,
        std::slice::from_ref(&residue),
        std::slice::from_ref(&residue),
        |path| {
            assert_eq!(path, residue);
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        },
        |directory| directory.sync_all(),
    );

    assert!(matches!(
        result,
        Err(ConnectionOperationError::Io { path, source })
            if path == residue && source.kind() == std::io::ErrorKind::PermissionDenied
    ));
    assert_eq!(fs::read(&residue).unwrap(), b"eligible but not unlinkable");
    assert_eq!(metadata_identity(&residue), residue_identity);
    assert_eq!(fs::read(fixture.journal.path()).unwrap(), canonical_before);
    assert_eq!(
        metadata_identity(fixture.journal.path()),
        canonical_identity
    );
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry));
}

// 첫 residue 삭제 뒤 두 번째 unlink가 실패해도 held parent fd를 한 번 sync한 다음 원래
// PermissionDenied를 반환하며, 두 번째 residue와 canonical intent는 그대로 보존합니다.
#[test]
fn partial_pending_residue_cleanup_is_synced_before_later_unlink_error() {
    let fixture = Fixture::new("partial-pending-cleanup-sync");
    let parent = fixture.journal.path().parent().unwrap();
    let mut guard = fixture.operation_guard();
    let entry = fixture.connect_entry();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    let canonical_before = fs::read(fixture.journal.path()).unwrap();
    let canonical_identity = metadata_identity(fixture.journal.path());
    let safe1 = pending_residue(parent, "77777777777777777777777777777777");
    let safe2 = pending_residue(parent, "88888888888888888888888888888888");
    write_mode(&safe1, b"first safe residue", 0o600);
    write_mode(&safe2, b"second safe residue", 0o600);
    let safe2_before = fs::read(&safe2).unwrap();
    let safe2_identity = metadata_identity(&safe2);
    let removal_order = RefCell::new(Vec::new());
    let sync_count = Cell::new(0_u8);

    let result = storage::cleanup_pending_residues_in_order_for_test(
        parent,
        &[safe1.clone(), safe2.clone()],
        &[safe1.clone(), safe2.clone()],
        |path| {
            removal_order.borrow_mut().push(path.to_owned());
            if path == safe1 {
                fs::remove_file(path)
            } else {
                assert_eq!(path, safe2);
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            }
        },
        |directory| {
            assert!(!safe1.exists());
            assert!(safe2.exists());
            sync_count.set(sync_count.get() + 1);
            directory.sync_all()
        },
    );

    assert!(matches!(
        result,
        Err(ConnectionOperationError::Io { path, source })
            if path == safe2 && source.kind() == std::io::ErrorKind::PermissionDenied
    ));
    assert_eq!(
        removal_order.into_inner(),
        vec![safe1.clone(), safe2.clone()]
    );
    assert_eq!(sync_count.get(), 1);
    assert!(!safe1.exists());
    assert_eq!(fs::read(&safe2).unwrap(), safe2_before);
    assert_eq!(metadata_identity(&safe2), safe2_identity);
    assert_eq!(fs::read(fixture.journal.path()).unwrap(), canonical_before);
    assert_eq!(
        metadata_identity(fixture.journal.path()),
        canonical_identity
    );
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry));
}

// 실제 chown이나 실행 권한에 기대지 않고 다른 expected euid를 주입하면 exact residue가
// WrongOwner로 거부되고 residue와 이미 게시된 canonical intent가 그대로 남습니다.
#[test]
fn wrong_owner_pending_residue_is_rejected_without_mutating_canonical_journal() {
    let fixture = Fixture::new("wrong-owner-pending");
    let parent = fixture.journal.path().parent().unwrap();
    let mut guard = fixture.operation_guard();
    let entry = fixture.connect_entry();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    let canonical_before = fs::read(fixture.journal.path()).unwrap();
    let canonical_identity = metadata_identity(fixture.journal.path());
    let residue = pending_residue(parent, "99999999999999999999999999999999");
    write_mode(&residue, b"owned by the current user", 0o600);
    let residue_before = fs::read(&residue).unwrap();
    let residue_identity = metadata_identity(&residue);
    let different_user = fs::symlink_metadata(&residue)
        .unwrap()
        .uid()
        .wrapping_add(1);

    assert!(matches!(
        storage::validate_pending_residue_owner_for_test(&residue, different_user),
        Err(ConnectionOperationError::WrongOwner(ref path)) if path == &residue
    ));
    assert_eq!(fs::read(&residue).unwrap(), residue_before);
    assert_eq!(metadata_identity(&residue), residue_identity);
    assert_eq!(fs::read(fixture.journal.path()).unwrap(), canonical_before);
    assert_eq!(
        metadata_identity(fixture.journal.path()),
        canonical_identity
    );
    assert_eq!(fixture.journal.capture().unwrap(), Some(entry));
}

// exact-name regular residue의 0200 mode와 상한+1 크기는 각각 typed failure를 내고
// unsafe residue 및 이미 게시된 canonical intent를 교체하거나 삭제하지 않습니다.
#[test]
fn wrong_mode_and_oversized_pending_residues_block_replacement_and_survive() {
    for (label, suffix, configure, expected) in [
        (
            "wrong-mode-pending",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            0_u8,
            "mode",
        ),
        (
            "oversized-pending",
            "cccccccccccccccccccccccccccccccc",
            1_u8,
            "size",
        ),
    ] {
        let fixture = Fixture::new(label);
        let parent = fixture.journal.path().parent().unwrap();
        let mut guard = fixture.operation_guard();
        let entry = fixture.connect_entry();
        fixture.journal.publish_intent(&mut guard, &entry).unwrap();
        let canonical_before = fs::read(fixture.journal.path()).unwrap();
        let canonical_identity = metadata_identity(fixture.journal.path());
        let residue = pending_residue(parent, suffix);
        if configure == 0 {
            write_mode(&residue, b"wrong mode", 0o200);
        } else {
            let file = fs::File::create(&residue).unwrap();
            file.set_len(MAX_OPERATION_JOURNAL_BYTES + 1).unwrap();
            fs::set_permissions(&residue, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let residue_identity = metadata_identity(&residue);

        let error = fixture
            .journal
            .advance(
                &mut guard,
                &entry,
                ConnectionOperationPhase::CredentialCommitted,
            )
            .unwrap_err();
        assert!(
            matches!((&error, expected),
                (ConnectionOperationError::InsecurePermissions(path), "mode") if path == &residue
            ) || matches!((&error, expected),
                (ConnectionOperationError::TooLarge(path), "size") if path == &residue
            ),
            "{label}: {error}"
        );
        assert_eq!(metadata_identity(&residue), residue_identity);
        assert_eq!(fs::read(fixture.journal.path()).unwrap(), canonical_before);
        assert_eq!(
            metadata_identity(fixture.journal.path()),
            canonical_identity
        );
        assert_eq!(fixture.journal.capture().unwrap(), Some(entry));
    }
}

// 비UTF-8 raw name은 byte grammar로 직접 거절하고, 길이·대문자·비hex·suffix가 틀린
// representable name은 위험한 file type/mode여도 cleanup하지 않고 inode를 보존합니다.
#[test]
fn malformed_and_unrelated_pending_names_remain_outside_cleanup_grammar() {
    let non_utf8 = OsString::from_vec(
        b".connection-operation.0000000000000000000000000000000\xff.pending".to_vec(),
    );
    assert!(!storage::pending_residue_name_for_test(&non_utf8));

    let fixture = Fixture::new("malformed-pending-names");
    let parent = fixture.journal.path().parent().unwrap();
    let mut guard = fixture.operation_guard();
    let first = fixture.connect_entry();
    fixture.journal.publish_intent(&mut guard, &first).unwrap();
    let canonical_before = fs::read(fixture.journal.path()).unwrap();
    let canonical_identity = metadata_identity(fixture.journal.path());

    let paths = [
        parent.join(".connection-operation.0000000000000000000000000000000.pending"),
        parent.join(".connection-operation.000000000000000000000000000000000.pending"),
        parent.join(".connection-operation.A0000000000000000000000000000000.pending"),
        parent.join(".connection-operation.g0000000000000000000000000000000.pending"),
        parent.join(".connection-operation.00000000000000000000000000000000"),
        parent.join(".connection-operation.00000000000000000000000000000000.pending.extra"),
        parent.join("unrelated-sentinel"),
    ];
    for path in &paths {
        assert!(!storage::pending_residue_name_for_test(
            path.file_name().unwrap()
        ));
    }
    write_mode(&paths[0], b"wrong mode but malformed", 0o400);
    fs::create_dir(&paths[1]).unwrap();
    let target = parent.join("malformed-symlink-target");
    write_mode(&target, b"target", 0o600);
    symlink(&target, &paths[2]).unwrap();
    for path in &paths[3..] {
        write_mode(path, b"untouched", 0o600);
    }
    let identities: Vec<_> = paths.iter().map(|path| metadata_identity(path)).collect();
    let eligible = pending_residue(parent, "66666666666666666666666666666666");
    write_mode(&eligible, b"eligible residue", 0o600);

    let second = fixture.connect_entry();
    assert!(matches!(
        fixture.journal.publish_intent(&mut guard, &second),
        Err(ConnectionOperationError::Conflict(ref path)) if path == fixture.journal.path()
    ));
    assert!(!eligible.exists());
    for (path, identity) in paths.iter().zip(identities) {
        assert_eq!(metadata_identity(path), identity, "{}", path.display());
    }
    assert_eq!(fs::read(fixture.journal.path()).unwrap(), canonical_before);
    assert_eq!(
        metadata_identity(fixture.journal.path()),
        canonical_identity
    );
    assert_eq!(fixture.journal.capture().unwrap(), Some(first));
}

// temporary publication과 cleanup grammar가 같은 production constructor/constants를 사용해
// 고정 16-byte random 값도 정확히 32 lowercase hex인 accepted raw name을 만듭니다.
#[test]
fn temporary_pending_name_constructor_matches_cleanup_grammar() {
    let parent = Path::new("constructor-parent");
    let path = storage::pending_residue_path_for_test(
        parent,
        [
            0x00, 0x1f, 0x20, 0x3a, 0x4b, 0x5c, 0x6d, 0x7e, 0x8f, 0x90, 0xab, 0xbc, 0xcd, 0xde,
            0xef, 0xff,
        ],
    );

    assert_eq!(path.parent(), Some(parent));
    assert!(storage::pending_residue_name_for_test(
        path.file_name().unwrap()
    ));
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
