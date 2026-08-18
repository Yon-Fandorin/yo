use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::model_service::{
    AccountId, ApiCredential, ConnectionOperationJournalEntry, LocalConnectionOperationGuard,
    LocalConnectionOperationJournal, LocalConnectionRepository, LocalCredentialRepository,
    ProviderId, StartupTarget,
};

pub(super) const CANDIDATE_SECRET: &str = "candidate-secret-must-not-enter-operation-journal";

const FIXTURE_ROOT_ATTEMPTS: usize = 64;
static NEXT_FIXTURE_ROOT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Fixture {
    root: PathBuf,
    pub(super) connections: LocalConnectionRepository,
    pub(super) credentials: LocalCredentialRepository,
    pub(super) journal: LocalConnectionOperationJournal,
}

impl Fixture {
    pub(super) fn new(name: &str) -> Self {
        let temp_dir = fs::canonicalize(std::env::temp_dir())
            .expect("the fixture temp directory must resolve to its physical path");
        let root = allocate_fixture_root(
            &temp_dir,
            name,
            FIXTURE_ROOT_ATTEMPTS,
            || NEXT_FIXTURE_ROOT.fetch_add(1, Ordering::Relaxed),
            |path| fs::create_dir(path),
        )
        .unwrap_or_else(|source| {
            panic!("the fixture root must be exclusively creatable: {source}")
        });
        Self {
            connections: LocalConnectionRepository::new(root.join("state/connections.yaml")),
            credentials: LocalCredentialRepository::new(root.join("state/credentials.yaml")),
            journal: LocalConnectionOperationJournal::new(
                root.join("state/connection-operation.yaml"),
            ),
            root,
        }
    }

    pub(super) fn connect_entry(&self) -> ConnectionOperationJournalEntry {
        let connection = self
            .connections
            .capture()
            .expect("the connection snapshot must be capturable")
            .prepare_preference(Some(StartupTarget::HostCodex))
            .expect("the public mutation must be preparable")
            .expect("the unset preference must produce a mutation");
        let credential = self
            .credentials
            .prepare_set(&provider(), &account())
            .expect("the credential mutation must be preparable");
        ConnectionOperationJournalEntry::connect_credential_change(connection, credential)
            .expect("the connect intent must be valid")
    }

    pub(super) fn operation_guard(&self) -> LocalConnectionOperationGuard {
        self.connections
            .acquire_operation()
            .expect("the fixture operation lock must be available")
    }

    pub(super) fn seed_disconnect_remove(&self) -> ConnectionOperationJournalEntry {
        let credential = self
            .credentials
            .prepare_set(&provider(), &account())
            .expect("the initial credential must be preparable");
        self.credentials
            .commit(
                &credential,
                Some(
                    &ApiCredential::new(CANDIDATE_SECRET)
                        .expect("the fixture credential must be valid"),
                ),
            )
            .expect("the initial credential must commit");
        let set = self
            .connections
            .capture()
            .expect("the initial public snapshot must be capturable")
            .prepare_preference(Some(StartupTarget::HostCodex))
            .expect("the initial public mutation must be preparable")
            .expect("the unset preference must produce a mutation");
        self.connections
            .commit(&set)
            .expect("the initial public mutation must commit");

        let connection = self
            .connections
            .capture()
            .expect("the disconnect public snapshot must be capturable")
            .prepare_preference(None)
            .expect("the disconnect public mutation must be preparable")
            .expect("the set preference must produce a removal mutation");
        let credential = self
            .credentials
            .prepare_remove(&provider(), &account())
            .expect("the credential removal must be preparable")
            .expect("the existing credential must produce a removal");
        ConnectionOperationJournalEntry::disconnect_remove(connection, credential)
            .expect("the disconnect intent must be valid")
    }

    pub(super) fn seed_disconnect_preserve(&self) -> ConnectionOperationJournalEntry {
        let credential = self
            .credentials
            .prepare_set(&provider(), &account())
            .expect("the preserved credential must be preparable");
        self.credentials
            .commit(
                &credential,
                Some(
                    &ApiCredential::new(CANDIDATE_SECRET)
                        .expect("the fixture credential must be valid"),
                ),
            )
            .expect("the preserved credential must commit");
        let set = self
            .connections
            .capture()
            .expect("the initial public snapshot must be capturable")
            .prepare_preference(Some(StartupTarget::HostCodex))
            .expect("the initial public mutation must be preparable")
            .expect("the unset preference must produce a mutation");
        self.connections
            .commit(&set)
            .expect("the initial public mutation must commit");
        let connection = self
            .connections
            .capture()
            .expect("the disconnect public snapshot must be capturable")
            .prepare_preference(None)
            .expect("the disconnect public mutation must be preparable")
            .expect("the set preference must produce a removal mutation");
        let expected_credential_revision = self
            .credentials
            .capture()
            .expect("the credential snapshot must be capturable")
            .revision()
            .clone();
        ConnectionOperationJournalEntry::disconnect_preserve(
            connection,
            expected_credential_revision,
        )
        .expect("the preserve intent must be valid")
    }
}

fn allocate_fixture_root(
    parent: &Path,
    name: &str,
    attempt_limit: usize,
    mut next_candidate: impl FnMut() -> u64,
    mut create_root: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    for _ in 0..attempt_limit {
        let root = fixture_root_candidate(parent, name, next_candidate());
        match create_root(&root) {
            Ok(()) => return Ok(root),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {},
            Err(source) => return Err(source),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("fixture root collision budget exhausted after {attempt_limit} attempts"),
    ))
}

fn fixture_root_candidate(parent: &Path, name: &str, candidate: u64) -> PathBuf {
    parent.join(format!(
        "yo-connection-operation-{}-{name}-{candidate}",
        std::process::id()
    ))
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn provider() -> ProviderId {
    ProviderId::new("qwencloud").expect("the fixture ProviderId must be valid")
}

pub(super) fn account() -> AccountId {
    AccountId::new("default").expect("the fixture AccountId must be valid")
}

pub(super) fn candidate() -> ApiCredential {
    ApiCredential::new(CANDIDATE_SECRET).expect("the fixture credential must be valid")
}

#[cfg(test)]
mod fixture_allocation_tests {
    use std::{cell::Cell, sync::Barrier, thread};

    use super::*;

    // 먼저 선택한 경로가 이미 존재하면 그 경로와 sentinel을 건드리지 않고 다음 번호의
    // 디렉터리를 배타적으로 생성해야 stale 경로와 PID 재사용이 fixture 소유권을 섞지 않는다.
    #[test]
    fn existing_candidate_is_preserved_and_allocation_uses_a_fresh_candidate() {
        let parent = Fixture::new("allocator-existing-parent");
        let stale_root = fixture_root_candidate(&parent.root, "retry", 41);
        fs::create_dir(&stale_root).expect("the stale candidate must be creatable");
        let sentinel = stale_root.join("sentinel");
        fs::write(&sentinel, b"foreign owner").expect("the sentinel must be writable");
        let candidate = Cell::new(41_u64);

        let allocated = allocate_fixture_root(
            &parent.root,
            "retry",
            2,
            || {
                let current = candidate.get();
                candidate.set(current.wrapping_add(1));
                current
            },
            |path| fs::create_dir(path),
        )
        .expect("the second candidate must be exclusively creatable");

        assert_eq!(allocated, fixture_root_candidate(&parent.root, "retry", 42));
        assert!(allocated.is_dir());
        assert_eq!(
            fs::read(&sentinel).expect("the foreign sentinel must remain readable"),
            b"foreign owner"
        );
        fs::remove_dir(&allocated).expect("the owned candidate must be removable");
        assert!(stale_root.is_dir());
        assert!(sentinel.is_file());
    }

    // 같은 label의 fixture 두 개를 동시에 만들어도 root와 세 저장소 경로가 모두 달라야 하며,
    // 하나를 Drop한 뒤에도 다른 fixture의 물리 경로와 저장소 동작은 유지되어야 한다.
    #[test]
    fn concurrent_same_label_fixtures_have_isolated_roots_and_drop_lifetimes() {
        let barrier = Barrier::new(3);
        let (first, second) = thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                Fixture::new("concurrent-same-label")
            });
            let second = scope.spawn(|| {
                barrier.wait();
                Fixture::new("concurrent-same-label")
            });
            barrier.wait();
            (
                first.join().expect("the first fixture must be created"),
                second.join().expect("the second fixture must be created"),
            )
        });
        let physical_temp = fs::canonicalize(std::env::temp_dir())
            .expect("the physical temp directory must resolve");

        assert_ne!(first.root, second.root);
        assert_eq!(first.root.parent(), Some(physical_temp.as_path()));
        assert_eq!(second.root.parent(), Some(physical_temp.as_path()));
        assert_ne!(first.connections.path(), second.connections.path());
        assert_ne!(first.credentials.path(), second.credentials.path());
        assert_ne!(first.journal.path(), second.journal.path());

        let second_entry = second.connect_entry();
        let mut second_guard = second.operation_guard();
        second
            .journal
            .publish_intent(&mut second_guard, &second_entry)
            .expect("the surviving fixture journal must accept an entry");
        let first_root = first.root.clone();
        let second_root = second.root.clone();
        drop(first);

        assert!(!first_root.exists());
        assert!(second_root.is_dir());
        assert_eq!(
            second
                .journal
                .capture()
                .expect("the surviving fixture journal must remain readable"),
            Some(second_entry)
        );
        second
            .connections
            .capture()
            .expect("the surviving connection repository must remain usable");
        second
            .credentials
            .capture()
            .expect("the surviving credential repository must remain usable");
    }

    // 충돌은 정해진 횟수만큼 서로 다른 후보로만 재시도하고, 그 밖의 I/O 오류는 첫 시도에서
    // 그대로 반환해야 테스트 실패가 무한 대기하거나 실제 파일시스템 오류를 숨기지 않는다.
    #[test]
    fn allocation_bounds_collision_retries_and_propagates_other_io_errors() {
        let parent = Path::new("fixture-allocation-test-parent");
        let candidate = Cell::new(u64::MAX - 1);
        let attempted = std::cell::RefCell::new(Vec::new());
        let exhausted = allocate_fixture_root(
            parent,
            "exhausted",
            3,
            || {
                let current = candidate.get();
                candidate.set(current.wrapping_add(1));
                current
            },
            |path| {
                attempted.borrow_mut().push(path.to_owned());
                Err(io::Error::from(io::ErrorKind::AlreadyExists))
            },
        )
        .expect_err("the collision budget must be finite");

        assert_eq!(exhausted.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(attempted.borrow().len(), 3);
        assert_ne!(attempted.borrow()[0], attempted.borrow()[1]);
        assert_ne!(attempted.borrow()[1], attempted.borrow()[2]);

        let create_calls = Cell::new(0_usize);
        let permission_denied = allocate_fixture_root(
            parent,
            "permission-denied",
            FIXTURE_ROOT_ATTEMPTS,
            || 7,
            |_| {
                create_calls.set(create_calls.get() + 1);
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
        )
        .expect_err("a non-collision error must fail immediately");
        assert_eq!(permission_denied.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(create_calls.get(), 1);
    }
}
