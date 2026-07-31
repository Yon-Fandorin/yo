use std::{
    fs::{self, OpenOptions},
    num::NonZeroU64,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    AppendError, DurableCutoff, DurableRecord, DurableRecordKind, LocalSessionRepository,
    RepositorySequence, SessionRepository, StoragePressureCause,
};
use crate::{JournalSequence, SessionId};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock should be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "yo-session-repository-{}-{name}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("the test directory should be created");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn session(value: u64) -> SessionId {
    SessionId::new(NonZeroU64::new(value).expect("test Session IDs are non-zero"))
}

// 재실행하면 기존 기록을 읽되 먼저 완전한 snapshot을 요구하고 다음 번호부터 이어 쓰는지 검증합니다.
#[test]
fn reopens_and_replays_an_ordered_session_log() {
    let directory = TestDirectory::new("reopen");
    let session_id = session(7);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        let first = repository
            .append(session_id, DurableRecord::incremental("first"))
            .expect("first append succeeds");
        let second = repository
            .append(session_id, DurableRecord::incremental("second"))
            .expect("second append succeeds");
        assert_eq!(first.sequence().get(), 1);
        assert_eq!(second.sequence().get(), 2);
    }

    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository reopens");
    assert!(matches!(
        repository.append(
            session_id,
            DurableRecord::incremental("unsafe continuation")
        ),
        Err(AppendError::SnapshotRequired { .. })
    ));
    let third = repository
        .append(session_id, DurableRecord::snapshot("complete state"))
        .expect("snapshot after reopen succeeds");
    let entries = repository
        .read_after(session_id, Some(RepositorySequence::new(1)), 8)
        .expect("suffix read succeeds");

    assert_eq!(third.sequence().get(), 3);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].record().payload(), "second");
    assert_eq!(entries[1].record().payload(), "complete state");
}

// writer lock을 가진 local repository가 기존 숫자 log 이름을 확인해 다음 Session ID를
// 배정하면 프로세스를 다시 시작해도 예전 Session 파일을 새 실행이 덮어쓰지 않는다.
#[test]
fn allocates_the_next_session_identity_after_existing_logs() {
    let directory = TestDirectory::new("next-session-id");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
    let first = repository
        .next_session_id()
        .expect("the first identity allocates");
    repository
        .append(first, DurableRecord::incremental("first session"))
        .expect("the first Session creates its log");

    let second = repository
        .next_session_id()
        .expect("the next identity observes the existing log");

    assert_eq!(first.get().get(), 1);
    assert_eq!(second.get().get(), 2);
}

// 용량 초과가 기존 파일을 바꾸지 않고 durable cutoff를 정확히 알려주는지 검증합니다.
#[test]
fn capacity_pressure_preserves_the_durable_prefix() {
    let directory = TestDirectory::new("capacity");
    let session_id = session(8);
    let mut repository =
        LocalSessionRepository::open(directory.path(), 320).expect("repository opens");
    repository
        .append(session_id, DurableRecord::incremental("small"))
        .expect("the first record fits");
    let before = fs::read(directory.path().join("8.jsonl")).expect("log exists");

    let error = repository
        .append(session_id, DurableRecord::incremental("x".repeat(256)))
        .expect_err("the second record exceeds capacity");
    let pressure = error
        .storage_pressure()
        .expect("capacity failure reports storage pressure");

    assert_eq!(pressure.cause(), StoragePressureCause::Capacity);
    assert_eq!(
        pressure.durable_cutoff(),
        DurableCutoff::Known {
            journal_sequence: None,
            repository_sequence: RepositorySequence::new(1),
        }
    );
    assert_eq!(
        fs::read(directory.path().join("8.jsonl")).expect("log remains readable"),
        before
    );
}

// 빈 로그를 정상적으로 확인한 뒤 용량이 부족한 경우에는 Unknown이 아니라
// `KnownEmpty`로 보고해 "확인된 빈 상태"와 "읽지 못한 상태"를 구분하는지 검증합니다.
#[test]
fn reports_a_known_empty_cutoff_before_the_first_record() {
    let directory = TestDirectory::new("known-empty-cutoff");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 0).expect("repository opens");

    let error = repository
        .append(session(20), DurableRecord::incremental("record"))
        .expect_err("zero capacity prevents the first record");
    let pressure = error
        .storage_pressure()
        .expect("capacity failure reports storage pressure");

    assert_eq!(pressure.durable_cutoff(), DurableCutoff::KnownEmpty);
}

// semantic Journal cutoff 5를 쓴 뒤 payload-only physical record가 하나 더 생겨도 두
// sequence를 서로 추론하지 않고 pressure가 Journal 5와 Repository 2를 함께 보고해야 합니다.
#[test]
fn reports_independent_journal_and_repository_cutoffs() {
    let directory = TestDirectory::new("independent-cutoffs");
    let session_id = session(23);
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
    repository
        .append(
            session_id,
            DurableRecord::incremental("semantic")
                .with_journal_cutoff(Some(JournalSequence::new(5))),
        )
        .expect("semantic record is durable");
    repository
        .append(session_id, DurableRecord::incremental("opaque audit"))
        .expect("non-Journal record shares the physical log");
    let used = fs::metadata(directory.path().join("23.jsonl"))
        .expect("log exists")
        .len();
    repository.set_capacity_bytes(used);

    let error = repository
        .append(session_id, DurableRecord::incremental("later"))
        .expect_err("no capacity remains");
    let pressure = error
        .storage_pressure()
        .expect("capacity failure reports both durable coordinates");

    assert_eq!(
        pressure.durable_cutoff(),
        DurableCutoff::Known {
            journal_sequence: Some(JournalSequence::new(5)),
            repository_sequence: RepositorySequence::new(2),
        }
    );
}

// 내구 기록이 끊긴 뒤에는 증분 레코드를 거부하고 완전한 스냅샷으로만 재개하는지 검증합니다.
#[test]
fn requires_a_snapshot_after_storage_pressure() {
    let directory = TestDirectory::new("snapshot-gate");
    let session_id = session(9);
    let mut repository =
        LocalSessionRepository::open(directory.path(), 320).expect("repository opens");
    repository
        .append(session_id, DurableRecord::incremental("prefix"))
        .expect("the prefix fits");
    repository
        .append(session_id, DurableRecord::incremental("x".repeat(256)))
        .expect_err("the oversized record creates a gap");

    assert!(matches!(
        repository.append(session_id, DurableRecord::incremental("later")),
        Err(AppendError::SnapshotRequired { .. })
    ));

    repository.set_capacity_bytes(32_768);
    let snapshot = repository
        .append(session_id, DurableRecord::snapshot("complete state"))
        .expect("a complete snapshot closes the gap");
    let later = repository
        .append(session_id, DurableRecord::incremental("later"))
        .expect("incremental persistence resumes after the snapshot");

    assert_eq!(snapshot.sequence().get(), 2);
    assert_eq!(later.sequence().get(), 3);
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
            .append(session_id, DurableRecord::incremental("durable"))
            .expect("the durable record is written");
    }
    let path = directory.path().join("10.jsonl");
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
        .append(session_id, DurableRecord::snapshot("after recovery"))
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
            .append(session_id, DurableRecord::incremental("durable"))
            .expect("the durable record is written");
    }
    let path = directory.path().join("13.jsonl");
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

// 완결된 줄이 손상되면 조용히 건너뛰지 않고 손상 위치를 보고하는지 검증합니다.
#[test]
fn rejects_a_corrupt_complete_line() {
    let directory = TestDirectory::new("corrupt-line");
    let path = directory.path().join("14.jsonl");
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
    fs::create_dir(directory.path().join("15.jsonl")).expect("a conflicting directory is created");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

    let error = repository
        .append(session(15), DurableRecord::incremental("record"))
        .expect_err("the conflicting path prevents append");
    let pressure = error
        .storage_pressure()
        .expect("filesystem failure reports storage pressure");

    assert_eq!(pressure.cause(), StoragePressureCause::Storage);
    assert_eq!(pressure.durable_cutoff(), DurableCutoff::Unknown);
}

// 같은 저장소 root를 두 writer가 동시에 열어 sequence와 용량 계산을 경쟁하지 못하게 하는지
// 검증합니다.
#[test]
fn allows_only_one_writer_for_a_repository_root() {
    let directory = TestDirectory::new("writer-lock");
    let first =
        LocalSessionRepository::open(directory.path(), 32_768).expect("first writer acquires lock");

    let error = LocalSessionRepository::open(directory.path(), 32_768)
        .expect_err("a second writer must not share the root");
    assert!(error.to_string().contains("another writer"));

    drop(first);
    LocalSessionRepository::open(directory.path(), 32_768)
        .expect("the lock is released with its owner");
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
            .append(session_id, DurableRecord::incremental("existing"))
            .expect("existing record is written");
    }
    let path = directory.path().join("16.jsonl");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000))
        .expect("fixture becomes temporarily unreadable");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository root opens");
    repository
        .append(session_id, DurableRecord::snapshot("unsafe"))
        .expect_err("the first load is unavailable");

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture becomes readable again");
    assert!(matches!(
        repository.append(session_id, DurableRecord::incremental("later")),
        Err(AppendError::SnapshotRequired {
            durable_cutoff: DurableCutoff::Known { .. }
        })
    ));
    let receipt = repository
        .append(session_id, DurableRecord::snapshot("complete state"))
        .expect("the retried load preserves the existing cutoff");

    assert_eq!(receipt.sequence().get(), 2);
}

// 기존 cutoff를 읽을 수 없었던 동안 메모리 실행이 계속될 수 있으므로, 저장소가 나중에
// 빈 로그를 읽더라도 증분 기록 대신 완전한 snapshot부터 요구하는지 검증합니다.
#[test]
fn requires_a_snapshot_after_an_initial_load_failure_recovers_to_an_empty_log() {
    let directory = TestDirectory::new("empty-load-retry");
    let session_id = session(18);
    let path = directory.path().join("18.jsonl");
    fs::create_dir(&path).expect("a conflicting directory makes the first load unavailable");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository root opens");

    repository
        .append(session_id, DurableRecord::incremental("not durable"))
        .expect_err("the first load is unavailable");
    fs::remove_dir(&path).expect("the transient conflict is removed");

    assert!(matches!(
        repository.append(session_id, DurableRecord::incremental("later")),
        Err(AppendError::SnapshotRequired {
            durable_cutoff: DurableCutoff::KnownEmpty
        })
    ));
    let receipt = repository
        .append(session_id, DurableRecord::snapshot("complete state"))
        .expect("a complete snapshot restarts durable history");
    assert_eq!(receipt.sequence().get(), 1);
}

// append 도중 rollback을 확인할 수 없다는 pending marker가 남으면 완결된 JSONL 줄도
// committed 기록으로 재생하지 않고 해당 세션 로그를 격리하는지 검증합니다.
#[test]
fn quarantines_a_complete_line_when_an_append_marker_remains() {
    let directory = TestDirectory::new("pending-append");
    let path = directory.path().join("19.jsonl");
    fs::write(
        &path,
        b"{\"schema\":\"yo.session-record/v1\",\"session_id\":19,\"sequence\":1,\"kind\":\"incremental\",\"payload\":\"uncertain\"}\n",
    )
    .expect("an ambiguous complete line is written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions are restricted");
    let pending = directory.path().join("19.jsonl.pending");
    fs::write(&pending, b"pending\n").expect("the durable pending marker is written");
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600))
        .expect("marker permissions are restricted");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository root opens");

    let read_error = repository
        .read_after(session(19), None, 8)
        .expect_err("an ambiguous complete line must not be replayed");
    assert!(read_error.to_string().contains("quarantined"));

    let append_error = repository
        .append(session(19), DurableRecord::snapshot("state"))
        .expect_err("a quarantined log must not accept another append");
    assert_eq!(
        append_error
            .storage_pressure()
            .map(|pressure| pressure.cause()),
        Some(StoragePressureCause::Storage)
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

// 다른 경로를 가리키는 symlink를 세션 로그로 따라가거나 수정하지 않는지 검증합니다.
#[test]
fn rejects_a_symbolic_link_at_a_session_log_path() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("symlink");
    let target = directory.path().join("target");
    fs::write(&target, b"unchanged").expect("target is written");
    symlink(&target, directory.path().join("17.jsonl")).expect("symlink is created");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

    let error = repository
        .append(session(17), DurableRecord::incremental("record"))
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
        .append(session_id, DurableRecord::snapshot("state"))
        .expect("record is written");

    let directory_mode = fs::metadata(directory.path())
        .expect("directory metadata exists")
        .permissions()
        .mode()
        & 0o777;
    let file_mode = fs::metadata(directory.path().join("11.jsonl"))
        .expect("file metadata exists")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(directory_mode, 0o700);
    assert_eq!(file_mode, 0o600);
}

// 와이어 형식이 버전, 세션, 순번, 종류를 명시해 향후 마이그레이션 근거를 남기는지 검증합니다.
#[test]
fn writes_an_explicit_versioned_jsonl_envelope() {
    let directory = TestDirectory::new("wire");
    let session_id = session(12);
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
    repository
        .append(session_id, DurableRecord::snapshot("state"))
        .expect("record is written");

    let contents = fs::read_to_string(directory.path().join("12.jsonl")).expect("log is readable");
    let value: serde_json::Value = serde_json::from_str(contents.trim()).expect("valid JSON");

    assert_eq!(value["schema"], "yo.session-record/v2");
    assert_eq!(value["session_id"], 12);
    assert_eq!(value["sequence"], 1);
    assert_eq!(value["kind"], "snapshot");
    assert_eq!(value["payload"], "state");
    assert_eq!(value["checksum"]["schema"], "crc32c/v1");
    assert_eq!(
        value["checksum"]["value"]
            .as_str()
            .expect("checksum is hexadecimal")
            .len(),
        8
    );
}

// 새 writer가 남긴 checksummed v2 record의 payload 한 글자만 바뀌어도 JSON 자체는
// 유효하지만 CRC32C가 달라지므로 replay 전에 complete-line 손상으로 거부해야 합니다.
#[test]
fn rejects_a_checksummed_record_whose_payload_was_changed() {
    let directory = TestDirectory::new("checksum-corruption");
    let session_id = session(21);
    {
        let mut repository =
            LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");
        repository
            .append(session_id, DurableRecord::incremental("alpha"))
            .expect("record is written");
    }
    let path = directory.path().join("21.jsonl");
    let contents = fs::read_to_string(&path).expect("log is readable");
    fs::write(&path, contents.replace("alpha", "omega")).expect("payload is tampered");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions remain restricted");
    let repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

    let error = repository
        .read_after(session_id, None, 8)
        .expect_err("checksum mismatch is corruption");

    assert!(error.to_string().contains("CRC32C"));
}

// checksummed v2 도입 뒤에도 지원 대상인 기존 v1 complete line은 checksum이 없다는
// 이유만으로 잃지 않고 동일한 opaque payload로 복구해야 합니다.
#[test]
fn continues_to_read_legacy_v1_records() {
    let directory = TestDirectory::new("legacy-v1");
    let path = directory.path().join("22.jsonl");
    fs::write(
        &path,
        b"{\"schema\":\"yo.session-record/v1\",\"session_id\":22,\"sequence\":1,\"kind\":\"incremental\",\"payload\":\"legacy\"}\n",
    )
    .expect("legacy fixture is written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("fixture permissions are restricted");
    let repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("repository opens");

    let entries = repository
        .read_after(session(22), None, 8)
        .expect("legacy record remains supported");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].record().payload(), "legacy");
}

// 테스트가 사용하는 레코드 종류도 실제 계약의 두 가지 값과 일치하는지 확인합니다.
#[test]
fn exposes_both_record_kinds_without_storage_details() {
    assert_eq!(
        DurableRecord::incremental("delta").kind(),
        DurableRecordKind::Incremental
    );
    assert_eq!(
        DurableRecord::snapshot("state").kind(),
        DurableRecordKind::Snapshot
    );
}
