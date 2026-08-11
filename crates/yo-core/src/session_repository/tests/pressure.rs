use std::fs;

use super::{
    super::{
        AppendError, DurableCutoff, DurableRecord, LocalSessionRepository, RepositorySequence,
        SessionRepository, StoragePressureCause,
    },
    support::{TestDirectory, discovered, log_path, session},
};
use crate::JournalSequence;

// 용량 초과가 기존 파일을 바꾸지 않고 durable cutoff를 정확히 알려주는지 검증합니다.
#[test]
fn capacity_pressure_preserves_the_durable_prefix() {
    let directory = TestDirectory::new("capacity");
    let session_id = session(8);
    let mut repository =
        LocalSessionRepository::open(directory.path(), 2_048).expect("repository opens");
    repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("small")),
        )
        .expect("the first record fits");
    let before = fs::read(log_path(directory.path(), session_id)).expect("log exists");

    let error = repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("x".repeat(4_096))),
        )
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
        fs::read(log_path(directory.path(), session_id)).expect("log remains readable"),
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
        .append(
            session(20),
            discovered(session(20), DurableRecord::incremental("record")),
        )
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
            discovered(
                session_id,
                DurableRecord::incremental("semantic")
                    .with_journal_cutoff(Some(JournalSequence::new(5))),
            ),
        )
        .expect("semantic record is durable");
    repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("opaque audit")),
        )
        .expect("non-Journal record shares the physical log");
    let used = fs::metadata(log_path(directory.path(), session_id))
        .expect("log exists")
        .len();
    repository.set_capacity_bytes(used);

    let error = repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("later")),
        )
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
        LocalSessionRepository::open(directory.path(), 2_048).expect("repository opens");
    repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("prefix")),
        )
        .expect("the prefix fits");
    repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("x".repeat(4_096))),
        )
        .expect_err("the oversized record creates a gap");

    assert!(matches!(
        repository.append(
            session_id,
            discovered(session_id, DurableRecord::incremental("later"))
        ),
        Err(AppendError::SnapshotRequired { .. })
    ));

    repository.set_capacity_bytes(32_768);
    let snapshot = repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::snapshot("complete state")),
        )
        .expect("a complete snapshot closes the gap");
    let later = repository
        .append(
            session_id,
            discovered(session_id, DurableRecord::incremental("later")),
        )
        .expect("incremental persistence resumes after the snapshot");

    assert_eq!(snapshot.sequence().get(), 2);
    assert_eq!(later.sequence().get(), 3);
}
