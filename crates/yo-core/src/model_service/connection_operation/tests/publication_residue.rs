use std::{
    cell::Cell,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use super::support::{Fixture, candidate};
use crate::model_service::{
    ConnectionOperationExecutionOutcome, ConnectionOperationJournalEntry,
    ConnectionRepositoryError, LocalConnectionOperationRepositories,
    connection_repository::{
        CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST, connection_temporary_path_for_test,
        create_connection_temporary_for_test,
    },
};

fn publish_connect_through_credential_commit(fixture: &Fixture) -> ConnectionOperationJournalEntry {
    let entry = fixture.connect_entry();
    let mut guard = fixture.operation_guard();
    fixture.journal.publish_intent(&mut guard, &entry).unwrap();
    fixture
        .credentials
        .commit(entry.credential_mutation().unwrap(), Some(&candidate()))
        .unwrap();
    drop(guard);
    entry
}

fn recover(
    fixture: &Fixture,
) -> Result<
    ConnectionOperationExecutionOutcome,
    crate::model_service::ConnectionOperationExecutionError,
> {
    LocalConnectionOperationRepositories::from_paths(
        fixture.connections.path(),
        fixture.credentials.path(),
        fixture.journal.path(),
    )
    .unwrap()
    .acquire()
    .unwrap()
    .recover_pending_operation()
}

fn legacy_pending_path(fixture: &Fixture, entry: &ConnectionOperationJournalEntry) -> PathBuf {
    let revision = entry.connection_mutation().planned_revision().to_string();
    let suffix = revision
        .strip_prefix("rev-")
        .expect("the planned revision must use the current token grammar");
    fixture
        .connections
        .path()
        .parent()
        .unwrap()
        .join(format!(".connections.{suffix}.pending"))
}

fn write_mode(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn identity(path: &Path) -> (u64, u64, u32, u64) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.len(),
    )
}

fn assert_recovery_completed_exact_publication(
    fixture: &Fixture,
    entry: &ConnectionOperationJournalEntry,
) {
    assert!(matches!(
        recover(fixture).unwrap(),
        ConnectionOperationExecutionOutcome::Completed { .. }
    ));
    assert_eq!(
        fixture.connections.capture().unwrap().revision(),
        entry.connection_mutation().planned_revision()
    );
    assert_eq!(
        fs::read(fixture.connections.path()).unwrap(),
        entry.connection_mutation().planned_bytes()
    );
    assert!(fixture.journal.capture().unwrap().is_none());
}

// 16-byte candidate는 planned revision과 무관한 정확한 32자리 lowercase hex suffix로
// 표현되어 같은 parent의 `.connections.<hex>.pending` 한 경로만 지목해야 합니다.
#[test]
fn generated_candidate_name_is_exact_lowercase_hex() {
    let parent = Path::new("/tmp/yo-connection-publication-name-test");
    let candidate = [
        0x00, 0x01, 0x0a, 0x0f, 0x10, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81, 0x92, 0xa3, 0xbc,
        0xff,
    ];

    assert_eq!(
        connection_temporary_path_for_test(parent, candidate),
        parent.join(".connections.00010a0f102b3c4d5e6f708192a3bcff.pending")
    );
}

// credential commit 뒤 public CAS 전에 남은 옛 deterministic regular residue가 있어도 실제
// recovery는 exact public bytes를 게시하고, 이름만 닮은 residue의 inode와 bytes는 보존합니다.
#[test]
fn exact_recovery_ignores_legacy_regular_publication_residue() {
    let fixture = Fixture::new("legacy-regular-publication-residue");
    let entry = publish_connect_through_credential_commit(&fixture);
    let residue = legacy_pending_path(&fixture, &entry);
    write_mode(&residue, b"foreign regular sentinel");
    let before = identity(&residue);

    assert_recovery_completed_exact_publication(&fixture, &entry);

    assert_eq!(identity(&residue), before);
    assert_eq!(fs::read(&residue).unwrap(), b"foreign regular sentinel");
}

// 옛 deterministic pending 이름이 symlink여도 실제 recovery는 그 link를 열거나 지우지 않고
// fresh temporary로 게시하여 link identity와 foreign target bytes를 모두 그대로 둡니다.
#[test]
fn exact_recovery_ignores_legacy_symlink_publication_residue() {
    let fixture = Fixture::new("legacy-symlink-publication-residue");
    let entry = publish_connect_through_credential_commit(&fixture);
    let parent = fixture.connections.path().parent().unwrap();
    let target = parent.join("foreign-symlink-target");
    write_mode(&target, b"foreign symlink target");
    let residue = legacy_pending_path(&fixture, &entry);
    symlink(&target, &residue).unwrap();
    let link_before = identity(&residue);

    assert_recovery_completed_exact_publication(&fixture, &entry);

    assert_eq!(identity(&residue), link_before);
    assert_eq!(fs::read_link(&residue).unwrap(), target);
    assert_eq!(fs::read(&target).unwrap(), b"foreign symlink target");
}

// 첫 generated candidate에 regular sentinel이 있으면 bytes를 새로 받아 두 번째 path만
// 배타 생성하고, 충돌한 sentinel의 inode와 bytes를 바꾸지 않아 foreign ownership을 지킵니다.
#[test]
fn generated_regular_collision_consumes_fresh_candidate_without_mutation() {
    let fixture = Fixture::new("generated-regular-collision");
    let guard = fixture.operation_guard();
    let parent = fixture.connections.path().parent().unwrap();
    let first = [0x11; 16];
    let second = [0x22; 16];
    let occupied = connection_temporary_path_for_test(parent, first);
    write_mode(&occupied, b"occupied regular candidate");
    let before = identity(&occupied);
    let mut candidates = [first, second].into_iter();

    let (created, file) = create_connection_temporary_for_test(
        parent,
        CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST,
        || {
            Ok(candidates
                .next()
                .expect("each attempt must request fresh bytes"))
        },
    )
    .unwrap();
    drop(file);

    assert_eq!(created, connection_temporary_path_for_test(parent, second));
    assert_eq!(identity(&occupied), before);
    assert_eq!(fs::read(&occupied).unwrap(), b"occupied regular candidate");
    fs::remove_file(created).unwrap();
    drop(guard);
}

// 첫 generated candidate가 symlink여도 사전 검사로 실패하지 않고 create_new 충돌로만
// 처리해 두 번째 path를 만들며, link와 target identity 및 bytes를 모두 보존합니다.
#[test]
fn generated_symlink_collision_retries_without_following_or_removing_link() {
    let fixture = Fixture::new("generated-symlink-collision");
    let guard = fixture.operation_guard();
    let parent = fixture.connections.path().parent().unwrap();
    let first = [0x33; 16];
    let second = [0x44; 16];
    let target = parent.join("generated-symlink-target");
    write_mode(&target, b"generated symlink target");
    let occupied = connection_temporary_path_for_test(parent, first);
    symlink(&target, &occupied).unwrap();
    let link_before = identity(&occupied);
    let target_before = identity(&target);
    let mut candidates = [first, second].into_iter();

    let (created, file) = create_connection_temporary_for_test(
        parent,
        CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST,
        || {
            Ok(candidates
                .next()
                .expect("each attempt must request fresh bytes"))
        },
    )
    .unwrap();
    drop(file);

    assert_eq!(created, connection_temporary_path_for_test(parent, second));
    assert_eq!(identity(&occupied), link_before);
    assert_eq!(identity(&target), target_before);
    assert_eq!(fs::read_link(&occupied).unwrap(), target);
    assert_eq!(fs::read(&target).unwrap(), b"generated symlink target");
    fs::remove_file(created).unwrap();
    drop(guard);
}

// entropy source가 첫 호출에서 실패하면 candidate path를 열지 않고 temporary-name 전용
// typed 오류와 정확한 진단을 반환해 revision 생성 실패로 오인되지 않게 합니다.
#[test]
fn entropy_failure_returns_before_any_candidate_open() {
    let fixture = Fixture::new("temporary-entropy-failure");
    let guard = fixture.operation_guard();
    let parent = fixture.connections.path().parent().unwrap();
    let calls = Cell::new(0_usize);

    let error = create_connection_temporary_for_test(
        parent,
        CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST,
        || {
            calls.set(calls.get() + 1);
            Err("injected temporary entropy failure".to_owned())
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ConnectionRepositoryError::TemporaryNameRandomness(ref message)
            if message == "injected temporary entropy failure"
    ));
    assert_eq!(
        error.to_string(),
        "generating a connection publication temporary name failed: injected temporary entropy failure"
    );
    assert_eq!(calls.get(), 1);
    assert!(!fs::read_dir(parent).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".connections.")
    }));
    drop(guard);
}

// candidate open이 AlreadyExists가 아닌 NotADirectory로 실패하면 난수 후보를 하나만
// 소비하고 exact failing path와 오류를 반환하여 unrelated I/O failure를 숨기지 않습니다.
#[test]
fn non_collision_open_failure_is_not_retried() {
    let fixture = Fixture::new("temporary-open-failure");
    let guard = fixture.operation_guard();
    let parent = fixture.connections.path().parent().unwrap();
    let not_directory = parent.join("not-a-directory");
    write_mode(&not_directory, b"regular parent substitute");
    let candidate = [0x55; 16];
    let calls = Cell::new(0_usize);

    let error = create_connection_temporary_for_test(
        &not_directory,
        CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST,
        || {
            calls.set(calls.get() + 1);
            Ok(candidate)
        },
    )
    .unwrap_err();

    let expected = connection_temporary_path_for_test(&not_directory, candidate);
    assert!(matches!(
        error,
        ConnectionRepositoryError::Io { ref path, ref source }
            if path == &expected && source.kind() == std::io::ErrorKind::NotADirectory
    ));
    assert_eq!(calls.get(), 1);
    assert_eq!(
        fs::read(&not_directory).unwrap(),
        b"regular parent substitute"
    );
    drop(guard);
}

// finite collision budget을 모두 소진하면 valid mutation 오류가 아니라 exact attempt 수를
// 담은 전용 typed 오류와 진단을 반환하고, occupied inode와 bytes를 전부 보존합니다.
#[test]
fn collision_exhaustion_consumes_exact_fresh_budget_without_mutation() {
    let fixture = Fixture::new("temporary-collision-exhaustion");
    let guard = fixture.operation_guard();
    let parent = fixture.connections.path().parent().unwrap();
    let candidates = (0..CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST)
        .map(|index| [u8::try_from(index).unwrap(); 16])
        .collect::<Vec<_>>();
    let occupied = candidates
        .iter()
        .map(|candidate| {
            let path = connection_temporary_path_for_test(parent, *candidate);
            let bytes = format!("occupied candidate {candidate:?}").into_bytes();
            write_mode(&path, &bytes);
            let before = identity(&path);
            (path, bytes, before)
        })
        .collect::<Vec<_>>();
    let calls = Cell::new(0_usize);

    assert_eq!(CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST, 2);

    let error = create_connection_temporary_for_test(
        parent,
        CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST,
        || {
            let index = calls.get();
            calls.set(index + 1);
            Ok(candidates[index])
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        ConnectionRepositoryError::TemporaryNameCollisionExhaustion {
            attempts: CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST
        }
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "all {CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST} generated connection publication temporary names already exist"
        )
    );
    assert_eq!(calls.get(), CONNECTION_TEMPORARY_ATTEMPTS_FOR_TEST);
    for (path, bytes, before) in occupied {
        assert_eq!(identity(&path), before);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    drop(guard);
}
