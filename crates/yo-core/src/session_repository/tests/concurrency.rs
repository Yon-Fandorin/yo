use std::{
    fs::{self, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    sync::{Arc, Barrier, mpsc},
    thread,
    time::Duration,
};

use super::{
    super::{
        DurableRecord, LocalSessionRepository, SessionRepository, StoragePressureCause,
        recover_stored_session_continuation,
    },
    support::{TestDirectory, discovered, log_path, session},
};

// 같은 root를 연 두 writer가 서로 다른 Session lease를 소유해 독립적으로 append하면서도
// 동일 Session에는 두 번째 writer가 들어오지 못하는지 검증합니다.
#[test]
fn allows_concurrent_root_writers_but_only_one_writer_per_session() {
    let directory = TestDirectory::new("writer-lock");
    let first_session = session(36);
    let second_session = session(37);
    let mut first =
        LocalSessionRepository::open(directory.path(), 32_768).expect("first root writer opens");
    let mut second =
        LocalSessionRepository::open(directory.path(), 32_768).expect("second root writer opens");

    first
        .append(
            first_session,
            discovered(first_session, DurableRecord::incremental("first")),
        )
        .expect("first Session acquires its lease");
    second
        .append(
            second_session,
            discovered(second_session, DurableRecord::incremental("second")),
        )
        .expect("another Session acquires an independent lease");

    let error = second
        .append(
            first_session,
            discovered(first_session, DurableRecord::snapshot("conflict")),
        )
        .expect_err("the second writer cannot acquire the first Session lease");
    assert!(error.to_string().contains("another writer owns Session"));

    drop(first);
    let receipt = second
        .append(
            first_session,
            discovered(first_session, DurableRecord::snapshot("successor")),
        )
        .expect("the exact Session lease is released with its owner");
    assert_eq!(receipt.sequence().get(), 2);
}

// 실행 가능한 continuation 복구가 physical history를 읽기 전에 Session writer lease를
// 얻어 유지하므로 두 process가 같은 Session을 함께 revalidate하지 못하는지 검증합니다.
#[test]
fn continuation_recovery_claims_the_session_before_reading() {
    let directory = TestDirectory::new("continuation-writer-lease");
    let session_id = session(41);
    {
        let mut seed =
            LocalSessionRepository::open(directory.path(), 32_768).expect("seed writer opens");
        seed.append(
            session_id,
            discovered(
                session_id,
                DurableRecord::incremental("not a Journal commit"),
            ),
        )
        .expect("the physical fixture is durable");
    }
    let mut first =
        LocalSessionRepository::open(directory.path(), 32_768).expect("first writer opens");
    let first_error = recover_stored_session_continuation(&mut first, session_id)
        .expect_err("the invalid semantic fixture fails after claiming the Session");
    assert!(!first_error.to_string().contains("another writer owns"));

    let mut second =
        LocalSessionRepository::open(directory.path(), 32_768).expect("second root writer opens");
    let competing_error = recover_stored_session_continuation(&mut second, session_id)
        .expect_err("a competing continuation cannot revalidate the owned Session");
    assert!(
        competing_error
            .to_string()
            .contains("another writer owns Session")
    );

    drop(first);
    let successor_error = recover_stored_session_continuation(&mut second, session_id)
        .expect_err("the successor reaches the same invalid semantic fixture");
    assert!(!successor_error.to_string().contains("another writer owns"));
}

// 구버전 writer가 root-exclusive `.writer.lock`을 보유하는 동안 신버전 repository가
// shared compatibility guard를 얻지 못해 혼합 버전의 동시 쓰기를 거부하는지 검증합니다.
#[test]
fn rejects_a_new_writer_while_a_legacy_writer_is_active() {
    let directory = TestDirectory::new("legacy-writer-blocks-new");
    let legacy_lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(directory.path().join(".writer.lock"))
        .expect("the legacy lock file opens");
    legacy_lock
        .lock()
        .expect("the simulated legacy writer owns the exclusive guard");

    let error = LocalSessionRepository::open(directory.path(), 32_768)
        .expect_err("the new writer must fail closed against a legacy writer");

    assert!(error.to_string().contains("legacy writer owns"));
}

// 신버전 repository의 lifetime shared compatibility guard가 구버전 방식의 exclusive
// `.writer.lock` 획득을 막되 다른 신버전 repository open은 허용하는지 검증합니다.
#[test]
fn prevents_a_legacy_writer_after_new_writers_open() {
    let directory = TestDirectory::new("new-writers-block-legacy");
    let first =
        LocalSessionRepository::open(directory.path(), 32_768).expect("first new writer opens");
    let second =
        LocalSessionRepository::open(directory.path(), 32_768).expect("second new writer opens");
    let legacy_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join(".writer.lock"))
        .expect("the compatibility lock file exists");

    assert!(matches!(
        legacy_lock.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));

    drop((first, second));
    legacy_lock
        .try_lock()
        .expect("the legacy exclusive guard is available after all new writers close");
}

// 서로 다른 Session writer가 동시에 마지막 용량 확인에 진입해도 root append coordinator가
// check와 append를 원자적으로 묶어 repository-wide 상한 안에서 하나만 성공시키는지 검증합니다.
#[test]
fn serializes_the_root_capacity_check_across_session_writers() {
    let first_session = session(39);
    let second_session = session(40);
    let measured_bytes = [first_session, second_session]
        .into_iter()
        .map(|session_id| {
            let probe = TestDirectory::new("capacity-probe");
            let mut repository = LocalSessionRepository::open(probe.path(), 32_768)
                .expect("the capacity probe opens");
            repository
                .append(
                    session_id,
                    discovered(session_id, DurableRecord::incremental("same payload")),
                )
                .expect("the probe record is durable");
            fs::metadata(log_path(probe.path(), session_id))
                .expect("the probe log exists")
                .len()
        })
        .max()
        .expect("two probe sizes exist");
    let capacity_bytes = measured_bytes.saturating_add(16);
    let directory = TestDirectory::new("concurrent-capacity");
    let first = LocalSessionRepository::open(directory.path(), capacity_bytes)
        .expect("first Session writer opens");
    let second = LocalSessionRepository::open(directory.path(), capacity_bytes)
        .expect("second Session writer opens");
    let barrier = Arc::new(Barrier::new(2));

    let workers =
        [(first, first_session), (second, second_session)].map(|(mut repository, session_id)| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                repository.append(
                    session_id,
                    discovered(session_id, DurableRecord::incremental("same payload")),
                )
            })
        });
    let results = workers.map(|worker| worker.join().expect("capacity worker does not panic"));
    let successful = results.iter().filter(|result| result.is_ok()).count();
    let capacity_limited = results
        .iter()
        .filter(|result| {
            result.as_ref().is_err_and(|error| {
                error
                    .storage_pressure()
                    .is_some_and(|pressure| pressure.cause() == StoragePressureCause::Capacity)
            })
        })
        .count();
    let durable_bytes = [first_session, second_session]
        .into_iter()
        .map(|session_id| {
            fs::metadata(log_path(directory.path(), session_id))
                .map_or(0, |metadata| metadata.len())
        })
        .sum::<u64>();

    assert_eq!(successful, 1);
    assert_eq!(capacity_limited, 1);
    assert!(durable_bytes <= capacity_bytes);
}

// 다른 owner가 `.append.lock`을 보유한 동안 append가 marker나 Session log를 만들지 않고
// 대기했다가 coordinator 해제 뒤에만 physical append를 수행하는지 검증합니다.
#[test]
fn physical_append_waits_for_the_root_coordinator() {
    let directory = TestDirectory::new("append-coordinator");
    let session_id = session(42);
    let coordinator = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(directory.path().join(".append.lock"))
        .expect("the coordinator file opens");
    coordinator
        .lock()
        .expect("the external owner holds the root coordinator");
    let mut repository =
        LocalSessionRepository::open(directory.path(), 32_768).expect("Session writer opens");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        entered_tx.send(()).expect("the worker announces append");
        let result = repository.append(
            session_id,
            discovered(session_id, DurableRecord::incremental("blocked")),
        );
        result_tx.send(result).expect("the worker reports append");
    });
    entered_rx.recv().expect("the worker starts");

    assert!(matches!(
        result_rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(!log_path(directory.path(), session_id).exists());

    coordinator.unlock().expect("the coordinator is released");
    let result = result_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("append completes after coordinator release");
    result.expect("the delayed append succeeds");
    worker.join().expect("the append worker does not panic");
}
