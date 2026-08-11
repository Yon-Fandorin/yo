use std::{
    fs::{self, OpenOptions},
    os::unix::fs::PermissionsExt,
};

use super::{
    super::{
        AppendError, DurableCutoff, DurableRecord, LocalSessionRepository, RepositorySequence,
        SessionRepository,
    },
    support::{TestDirectory, discovered, log_path, session},
};

// 재실행하면 기존 기록을 읽되 먼저 완전한 snapshot을 요구하고 다음 번호부터 이어 쓰는지 검증합니다.
#[test]
fn reopens_and_replays_an_ordered_session_log() {
    let directory = TestDirectory::new("reopen");
    let session_id = session(7);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        let first = repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("first")),
            )
            .expect("first append succeeds");
        let second = repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("second")),
            )
            .expect("second append succeeds");
        assert_eq!(first.sequence().get(), 1);
        assert_eq!(second.sequence().get(), 2);
    }

    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository reopens");
    assert!(matches!(
        repository.append(
            session_id,
            discovered(
                session_id,
                DurableRecord::incremental("unsafe continuation")
            )
        ),
        Err(AppendError::SnapshotRequired { .. })
    ));
    let third = repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("complete state")),
        )
        .expect("snapshot after reopen succeeds");
    let entries = repository
        .read_after(session_id, Some(RepositorySequence::new(1)), 8)
        .expect("suffix read succeeds");

    assert_eq!(third.sequence().get(), 3);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].record().payload(), "second");
    assert_eq!(entries[1].record().payload(), "complete state");
}

// 충돌로 잘린 마지막 JSONL 줄을 미커밋 꼬리로 버리고 안전한 prefix에서 재개하는지 검증합니다.
#[test]
fn repairs_an_incomplete_final_line_before_appending() {
    let directory = TestDirectory::new("partial-tail");
    let session_id = session(10);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("durable")),
            )
            .expect("the durable record is written");
    }
    let path = log_path(directory.path(), session_id);
    use std::io::Write as _;
    OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("log opens")
        .write_all(br#"{"schema":"yo.session-record/v1""#)
        .expect("partial tail is simulated");

    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository reopens");
    repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("after recovery")),
        )
        .expect("snapshot repairs the partial tail");
    let entries = repository
        .read_after(session_id, None, 8)
        .expect("the repaired log reads");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1].record().payload(), "after recovery");
}

// 읽기 전용 접근은 미커밋 꼬리를 무시하되 파일 자체를 수정하지 않는지 검증합니다.
#[test]
fn read_only_replay_does_not_repair_the_physical_file() {
    let directory = TestDirectory::new("read-only-tail");
    let session_id = session(13);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("durable")),
            )
            .expect("the durable record is written");
    }
    let path = log_path(directory.path(), session_id);
    use std::io::Write as _;
    OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("log opens")
        .write_all(b"{")
        .expect("partial tail is simulated");
    let before = fs::metadata(&path).expect("metadata exists").len();

    let repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository reopens");
    let entries = repository
        .read_after(session_id, None, 8)
        .expect("the durable prefix reads");

    assert_eq!(entries.len(), 1);
    assert_eq!(fs::metadata(path).expect("metadata exists").len(), before);
}

// 첫 load의 일시적 실패가 빈 state로 남지 않고 다음 시도에서 기존 cutoff를 다시 읽는지 검증합니다.
#[test]
fn retries_session_state_after_a_transient_load_failure() {
    let directory = TestDirectory::new("load-retry");
    let session_id = session(16);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        repository
            .append(
                session_id,
                discovered(session_id, DurableRecord::incremental("existing")),
            )
            .expect("existing record is written");
    }
    let path = log_path(directory.path(), session_id);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000))
        .expect("fixture becomes temporarily unreadable");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository root opens");
    repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("unsafe")),
        )
        .expect_err("the first load is unavailable");

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture becomes readable again");
    assert!(matches!(
        repository.append(
            session_id,
            discovered(session_id, DurableRecord::incremental("later"))
        ),
        Err(AppendError::SnapshotRequired {
            durable_cutoff: DurableCutoff::Known { .. }
        })
    ));
    let receipt = repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("complete state")),
        )
        .expect("the retried load preserves the existing cutoff");

    assert_eq!(receipt.sequence().get(), 2);
}

// 기존 cutoff를 읽을 수 없었던 동안 메모리 실행이 계속될 수 있으므로, 저장소가 나중에
// 빈 로그를 읽더라도 증분 기록 대신 완전한 snapshot부터 요구하는지 검증합니다.
#[test]
fn requires_a_snapshot_after_an_initial_load_failure_recovers_to_an_empty_log() {
    let directory = TestDirectory::new("empty-load-retry");
    let session_id = session(18);
    let path = log_path(directory.path(), session_id);
    fs::create_dir(&path).expect("a conflicting directory makes the first load unavailable");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository root opens");

    repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("not durable")),
        )
        .expect_err("the first load is unavailable");
    fs::remove_dir(&path).expect("the transient conflict is removed");

    assert!(matches!(
        repository.append(
            session_id,
            discovered(session_id, DurableRecord::incremental("later"))
        ),
        Err(AppendError::SnapshotRequired {
            durable_cutoff: DurableCutoff::KnownEmpty
        })
    ));
    let receipt = repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("complete state")),
        )
        .expect("a complete snapshot restarts durable history");
    assert_eq!(receipt.sequence().get(), 1);
}
