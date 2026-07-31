use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{capture_index_from, ensure_index_unchanged, read_index_blob};

static REPOSITORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const OPERATION: &str = "test_proposal_index";

struct TestRepository {
    root: PathBuf,
}

impl TestRepository {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "methexis-proposal-index-{}-{}",
            std::process::id(),
            REPOSITORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let repository = Self { root };
        repository.git(&["init", "-q"]);
        repository.git(&["config", "user.name", "Methexis Test"]);
        repository.git(&["config", "user.email", "methexis@example.invalid"]);
        repository.write("candidate.txt", b"trusted\n");
        repository.git(&["add", "candidate.txt"]);
        repository.git(&["commit", "-q", "-m", "initialize fixture"]);
        repository
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        fs::write(self.root.join(path), bytes).unwrap();
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("/usr/bin/git")
            .env_clear()
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git command failed: {args:?}");
    }
}

impl Drop for TestRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn stage(repository: &TestRepository, bytes: &[u8]) {
    repository.write("candidate.txt", bytes);
    repository.git(&["add", "candidate.txt"]);
}

fn root(repository: &TestRepository) -> &Path {
    &repository.root
}

// capture 뒤 live index가 바뀌어도 후보 blob은 최초 tree OID에서 읽어 중간 상태를
// 섞지 않고, 마지막 guard는 실제 commit 대상이 달라졌음을 명시적으로 거부한다.
#[test]
fn proposal_snapshot_pins_blob_reads_and_detects_a_later_index_change() {
    let repository = TestRepository::new();
    stage(&repository, b"candidate-a\n");
    let index = capture_index_from(root(&repository), None, OPERATION).unwrap();

    stage(&repository, b"candidate-b\n");

    assert_eq!(
        read_index_blob(root(&repository), &index, "candidate.txt", OPERATION).unwrap(),
        b"candidate-a\n"
    );
    let failure = ensure_index_unchanged(root(&repository), &index, OPERATION).unwrap_err();
    assert_eq!(failure.code(), "staged_candidate_changed_during_validation");
}
