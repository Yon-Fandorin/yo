use std::{fs, os::unix::fs::PermissionsExt};

use super::{
    super::{
        AppendError, DurableCutoff, DurableRecord, LocalSessionReader, LocalSessionRepository,
        SessionRepository, StoragePressureCause, StoredSessionReader,
    },
    support::{TestDirectory, discovered, log_path, session},
};

// 완결된 줄이 손상되면 조용히 건너뛰지 않고 손상 위치를 보고하는지 검증합니다.
#[test]
fn rejects_a_corrupt_complete_line() {
    let directory = TestDirectory::new("corrupt-line");
    let session_id = session(14);
    let path = log_path(directory.path(), session_id);
    fs::write(&path, b"{not json}\n").expect("corrupt fixture is written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions are restricted");
    let repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

    let error = repository
        .read_after(session(14), None, 8)
        .expect_err("a complete corrupt line must not be ignored");

    assert!(error.to_string().contains("line 1"));
}

// 최초 로그를 읽을 수 없는 파일시스템 실패를 typed storage pressure로 보고하면서,
// 확인하지 못한 cutoff를 빈 로그로 단정하지 않고 Unknown으로 보존하는지 검증합니다.
#[test]
fn classifies_filesystem_append_failures_as_storage_pressure() {
    let directory = TestDirectory::new("storage-error");
    let session_id = session(15);
    fs::create_dir(log_path(directory.path(), session_id))
        .expect("a conflicting directory is created");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

    let error = repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("record")),
        )
        .expect_err("the conflicting path prevents append");
    let pressure = error
        .storage_pressure()
        .expect("filesystem failure reports storage pressure");

    assert_eq!(pressure.cause(), StoragePressureCause::Storage);
    assert_eq!(pressure.durable_cutoff(), DurableCutoff::Unknown);
}

// append 도중 rollback을 확인할 수 없다는 pending marker가 남으면 완결된 JSONL 줄도
// committed 기록으로 재생하지 않고 reader와 후속 writer가 모두 격리하는지 검증합니다.
#[test]
fn quarantines_a_complete_line_when_an_append_marker_remains() {
    let directory = TestDirectory::new("pending-append");
    let session_id = session(19);
    let path = log_path(directory.path(), session_id);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("uncertain")),
            )
            .expect("a complete v1 line is written");
    }
    let pending = path.with_extension("jsonl.pending");
    fs::write(&pending, b"pending\n").expect("the durable pending marker is written");
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600))
        .expect("marker permissions are restricted");
    let reader = LocalSessionReader::open(directory.path()).expect("read-only root opens");
    let read_error = reader
        .read_after(session_id, None, 8)
        .expect_err("an ambiguous complete line must not be replayed");
    assert!(matches!(
        read_error,
        crate::session_repository::RepositoryError::Quarantined { .. }
    ));

    let mut successor = LocalSessionRepository::open(directory.path(), 32_768)
        .expect("an abandoned marker does not quarantine another Session");
    let append_error = successor
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("unsafe")),
        )
        .expect_err("a successor writer must not adopt the abandoned Session marker");
    assert!(matches!(
        append_error,
        AppendError::StoragePressure {
            source: Some(crate::session_repository::RepositoryError::Quarantined { .. }),
            ..
        }
    ));
}

// `..`가 포함된 입력 경로도 open 시점에 절대 경로로 고정해 이후 현재 디렉터리 변화가
// writer lock과 실제 append 대상을 서로 다른 root로 돌리지 못하게 하는지 검증합니다.
#[test]
fn resolves_the_repository_root_once_when_opening() {
    let directory = TestDirectory::new("stable-root");
    let unresolved = directory.path().join("..").join(
        directory
            .path()
            .file_name()
            .expect("the test directory has a name"),
    );
    let repository =
        LocalSessionRepository::open(&unresolved, 32_768).expect("repository root opens");

    assert!(repository.root_path().is_absolute());
    assert_eq!(
        repository.root_path(),
        fs::canonicalize(directory.path()).expect("the root canonicalizes")
    );
}

// 다른 경로를 가리키는 symlink를 세션 로그로 따라가거나 수정하지 않는지 검증합니다.
#[test]
fn rejects_a_symbolic_link_at_a_session_log_path() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("symlink");
    let session_id = session(17);
    let target = directory.path().join("target");
    fs::write(&target, b"unchanged").expect("target is written");
    symlink(&target, log_path(directory.path(), session_id)).expect("symlink is created");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

    let error = repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("record")),
        )
        .expect_err("the symlink must not be followed");

    assert_eq!(
        error.storage_pressure().map(|pressure| pressure.cause()),
        Some(StoragePressureCause::Storage)
    );
    assert_eq!(
        fs::read(target).expect("target remains readable"),
        b"unchanged"
    );
}

// 로컬 저장 디렉터리와 세션 파일을 현재 사용자만 읽고 쓸 수 있게 제한하는지 검증합니다.
#[test]
fn restricts_repository_permissions_to_the_current_user() {
    let directory = TestDirectory::new("permissions");
    let session_id = session(11);
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
    repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("state")),
        )
        .expect("record is written");

    let directory_mode = fs::metadata(directory.path())
        .expect("directory metadata exists")
        .permissions()
        .mode()
        & 0o777;
    let file_mode = fs::metadata(log_path(directory.path(), session_id))
        .expect("file metadata exists")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(directory_mode, 0o700);
    assert_eq!(file_mode, 0o600);
}
