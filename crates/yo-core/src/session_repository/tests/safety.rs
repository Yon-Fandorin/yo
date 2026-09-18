use std::{
    env, fs,
    os::unix::fs::{PermissionsExt, symlink},
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

use super::{
    super::{
        AppendError, DurableCutoff, DurableRecord, LocalSessionReader, LocalSessionRepository,
        SessionRepository, StoragePressureCause, StoredSession, StoredSessionReader,
        StoredSessionUnavailableReason,
    },
    support::{TestDirectory, discovered, log_path, session},
};
#[cfg(test)]
use crate::session_repository::RepositoryError;

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
    assert!(matches!(read_error, RepositoryError::Quarantined { .. }));

    let mut successor = LocalSessionRepository::open(directory.path(), 32_768)
        .expect("an abandoned marker does not quarantine another Session");
    let append_error = successor
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("unsafe")),
        )
        .expect_err("a successor writer must not adopt the abandoned Session marker");
    assert!(
        matches!(
            &append_error,
            AppendError::StoragePressure {
                source: Some(RepositoryError::Quarantined { .. }),
                ..
            }
        ),
        "unexpected append error: {append_error:?}"
    );
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

// reader와 writer 모두 빈 경로·상대 경로를 filesystem access 전에 같은 typed error로 거절한다.
#[test]
fn rejects_empty_and_relative_repository_roots_for_both_openers() {
    let roots = ["", ".", "./relative", "relative"];
    for root in roots {
        let reader_error = LocalSessionReader::open(root)
            .expect_err("a reader must reject a non-absolute repository root");
        assert!(matches!(
            reader_error,
            RepositoryError::Unavailable { message }
                if message == "Session repository root must be a non-empty absolute path"
        ));

        let writer_error = LocalSessionRepository::open(root, 32_768)
            .expect_err("a writer must reject a non-absolute repository root");
        assert!(matches!(
            writer_error,
            RepositoryError::Unavailable { message }
                if message == "Session repository root must be a non-empty absolute path"
        ));
    }
}

// absolute 경로의 dot segment는 입력 계약을 만족하므로 두 opener가 canonical root로 고정한다.
#[test]
fn accepts_absolute_dot_segments_and_canonicalizes_for_both_openers() {
    let directory = TestDirectory::new("absolute-dot-segments");
    let name = directory
        .path()
        .file_name()
        .expect("the test directory has a name");
    let unresolved = directory.path().join(".").join("..").join(name);
    let canonical = fs::canonicalize(directory.path()).expect("the root canonicalizes");

    let repository =
        LocalSessionRepository::open(&unresolved, 32_768).expect("the writer root opens");
    assert_eq!(repository.root_path(), canonical);
    drop(repository);

    let reader = LocalSessionReader::open(&unresolved).expect("the reader root opens");
    assert_eq!(reader.root_path(), canonical);
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

// reader가 세션 로그 경로의 symlink를 따라가지 않고 대상 파일을 읽거나 변경하지 않는 경계
#[test]
fn reader_rejects_a_symbolic_link_at_a_session_log_path() {
    let directory = TestDirectory::new("reader-symlink");
    let session_id = session(18);
    let path = log_path(directory.path(), session_id);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::snapshot("record")),
            )
            .expect("the session log is written");
    }

    let target = directory.path().join("target");
    fs::copy(&path, &target).expect("the external target is copied");
    let target_contents = fs::read(&target).expect("the external target is readable");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
        .expect("target permissions are restricted");
    fs::remove_file(&path).expect("the session log is removed");
    symlink(&target, &path).expect("the session log symlink is created");

    let reader = LocalSessionReader::open(directory.path()).expect("reader opens");
    let error = reader
        .read_after(session_id, None, 8)
        .expect_err("the reader must reject a session log symlink");

    assert!(matches!(error, RepositoryError::Unavailable { .. }));
    assert_eq!(
        fs::read(&target).expect("target remains readable"),
        target_contents
    );
}

// pending marker의 symlink는 discovery와 snapshot read 모두에서 외부 대상을 따라가지 않고
// 해당 Session만 unreadable 상태로 남기는 경계
#[test]
fn reader_rejects_a_symbolic_link_at_a_pending_marker_path() {
    let directory = TestDirectory::new("pending-symlink");
    let session_id = session(20);
    let path = log_path(directory.path(), session_id);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::snapshot("record")),
            )
            .expect("the session log is written");
    }
    let target = directory.path().join("pending-target");
    fs::write(&target, b"0\n").expect("the external marker target is written");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
        .expect("target permissions are restricted");
    let pending = path.with_extension("jsonl.pending");
    symlink(&target, &pending).expect("the pending marker symlink is created");

    let reader = LocalSessionReader::open(directory.path()).expect("reader opens");
    let sessions = reader.discover().expect("discovery remains available");
    assert!(matches!(
        sessions.as_slice(),
        [StoredSession::Unavailable {
            session_id: observed,
            reason: StoredSessionUnavailableReason::Unreadable { .. },
        }] if *observed == session_id
    ));
    assert!(matches!(
        reader.read_after(session_id, None, 8),
        Err(RepositoryError::Unavailable { .. })
    ));
    assert_eq!(fs::read(&target).unwrap(), b"0\n");
}

// append가 root를 고정한 직후 pathname이 교체되어도 log, pending marker, directory sync가
// 모두 같은 열린 tree에 머물고 replacement tree에는 아무 entry도 만들지 않는 경계
#[test]
fn append_keeps_log_and_marker_in_the_pinned_root_after_path_replacement() {
    let directory = TestDirectory::new("append-pinned-root");
    let root = directory.path().to_owned();
    let moved = root.with_extension("moved");
    let mut repository =
        LocalSessionRepository::open(&root, 32_768).expect("repository opens before replacement");
    let hook_root = root.clone();
    let hook_moved = moved.clone();
    super::super::local::install_append_root_pinned_hook(move || {
        fs::rename(&hook_root, &hook_moved).expect("the opened repository root is moved");
        fs::create_dir(&hook_root).expect("a replacement repository root is created");
        fs::set_permissions(&hook_root, fs::Permissions::from_mode(0o700))
            .expect("replacement root permissions are restricted");
    });
    repository
        .append(
            session(23),
            discovered(session(23), DurableRecord::snapshot("record")),
        )
        .expect("append stays on the pinned repository root");
    drop(repository);

    assert!(fs::metadata(log_path(&moved, session(23))).unwrap().len() > 0);
    assert!(
        !log_path(&moved, session(23))
            .with_extension("jsonl.pending")
            .exists()
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);

    fs::remove_dir(&root).expect("the replacement repository root is removed");
    fs::rename(&moved, &root).expect("the fixture repository root is restored");
}

fn wait_bounded(mut child: Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            outcome => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Session repository test child did not finish: {outcome:?}");
            },
        }
    }
}

// 작성자가 없는 session log와 pending marker FIFO를 discovery와 snapshot reader가
// 기다리지 않고 제한 시간 안에 unavailable로 거부하는 경계
#[test]
fn reader_rejects_session_and_pending_fifos_without_blocking() {
    const CHILD_ROOT: &str = "YO_SESSION_REPOSITORY_FIFO_TEST_ROOT";
    if let Some(root) = env::var_os(CHILD_ROOT) {
        let reader = LocalSessionReader::open(root).expect("reader opens the child fixture");
        let sessions = reader.discover().expect("FIFO discovery remains bounded");
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().all(|session| matches!(
            session,
            StoredSession::Unavailable {
                reason: StoredSessionUnavailableReason::Unreadable { .. },
                ..
            }
        )));
        assert!(matches!(
            reader.read_after(session(21), None, 8),
            Err(RepositoryError::Unavailable { .. })
        ));
        assert!(matches!(
            reader.read_after(session(22), None, 8),
            Err(RepositoryError::Unavailable { .. })
        ));
        return;
    }

    let directory = TestDirectory::new("reader-fifos");
    let pending_session = session(21);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        repository
            .append(
                pending_session,
                discovered(pending_session, DurableRecord::snapshot("record")),
            )
            .expect("the session log is written");
    }
    assert!(
        Command::new("mkfifo")
            .args(["-m", "600"])
            .arg(log_path(directory.path(), pending_session).with_extension("jsonl.pending"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("mkfifo")
            .args(["-m", "600"])
            .arg(log_path(directory.path(), session(22)))
            .status()
            .unwrap()
            .success()
    );
    let child = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "session_repository::tests::safety::reader_rejects_session_and_pending_fifos_without_blocking",
        ])
        .env(CHILD_ROOT, directory.path())
        .spawn()
        .unwrap();
    assert!(wait_bounded(child).success());
}
