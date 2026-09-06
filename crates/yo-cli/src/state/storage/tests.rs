use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{open_at, open_host_identity_at, open_reader_at};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is after the Unix epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "yo-cli-storage-{}-{name}-{nonce}",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// connect 검증은 Session repository를 만들지 않고 live startup과 같은 state-root의 stable
// WorkspaceHostId를 재사용해야 두 진입점이 서로 다른 Host identity를 만들지 않습니다.
#[test]
fn host_identity_can_open_without_a_session_repository() {
    let directory = TestDirectory::new("host-only");

    let first = open_host_identity_at(directory.path().to_owned()).unwrap();
    let second = open_host_identity_at(directory.path().to_owned()).unwrap();

    assert_eq!(first, second);
    assert!(directory.path().join("host/host-id").is_file());
    assert!(!directory.path().join("sessions").exists());
}

// Session repository를 다른 디스크나 원격 mount로 옮겨도 Host의 정체성은 플랫폼
// 상태 루트가 소유하므로, 서로 다른 repository가 같은 Host ID를 받는지 검증합니다.
#[test]
fn repository_relocation_does_not_change_workspace_host_identity() {
    let directory = TestDirectory::new("relocation");
    let state_root = directory.path().join("state");

    let first = open_at(
        state_root.clone(),
        directory.path().join("repository-a"),
        4096,
    )
    .unwrap();
    let (_, first_host_id) = first.into_parts();
    let second = open_at(state_root, directory.path().join("repository-b"), 4096).unwrap();
    let (_, second_host_id) = second.into_parts();

    assert_eq!(second_host_id, first_host_id);
}

// 이전 Yo가 sessions만 0700으로 만들고 그 부모 state root를 일반 0755로 남긴
// 상태에서도, 별도 0700 host 경계를 만들어 기존 저장소와 호환되는지 검증합니다.
#[test]
fn existing_repository_parent_does_not_block_host_identity_creation() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new("existing-parent");
    let state_root = directory.path().join("state");
    let repository_root = state_root.join("sessions");
    fs::create_dir_all(&repository_root).unwrap();
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(&repository_root, fs::Permissions::from_mode(0o700)).unwrap();

    let storage = open_at(state_root.clone(), repository_root, 4096).unwrap();
    let (_, host_id) = storage.into_parts();

    assert_eq!(
        fs::metadata(state_root.join("host"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert!(
        fs::read_to_string(state_root.join("host/host-id"))
            .unwrap()
            .contains(&host_id.to_string())
    );
}

// 읽기 전용 Session 명령을 새 머신 상태에 실행하면 writer용 host identity나 repository
// 디렉터리를 만들지 않고 둘 다 없는 snapshot으로 성공해야 조회가 상태를 변경하지 않는다.
#[test]
fn read_only_storage_open_does_not_create_missing_paths() {
    let directory = TestDirectory::new("read-only-missing");
    let state_root = directory.path().join("state");
    let repository_root = directory.path().join("sessions");

    let storage = open_reader_at(state_root.clone(), repository_root.clone()).unwrap();

    assert!(storage.reader().is_none());
    assert!(storage.workspace_host_id().is_none());
    assert!(!state_root.exists());
    assert!(!repository_root.exists());
}
