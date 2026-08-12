use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    DEFAULT_CAPACITY_BYTES, capacity_bytes_from, open_at, open_host_identity_at, open_reader_at,
    platform_state_root_from, repository_root_from,
};

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

// 명시적인 repository override는 OS 기본 위치보다 먼저 선택되어 test와 운영자가
// 같은 단일 writer root를 의도적으로 지정할 수 있어야 한다.
#[test]
fn explicit_repository_root_has_priority() {
    let state_root = PathBuf::from("/tmp/xdg/yo");
    let root = repository_root_from(Some(OsString::from("/tmp/yo-explicit")), &state_root).unwrap();

    assert_eq!(root, PathBuf::from("/tmp/yo-explicit"));
}

// capacity 환경값이 없으면 제품 기본 1 GiB를 사용하고, 숫자가 아닌 값은 조용히
// fallback하지 않아 사용자가 잘못된 저장 한도를 즉시 알 수 있어야 한다.
#[test]
fn capacity_uses_the_default_and_rejects_invalid_input() {
    assert_eq!(capacity_bytes_from(None).unwrap(), DEFAULT_CAPACITY_BYTES);
    assert!(capacity_bytes_from(Some(OsString::from("1GiB"))).is_err());
    assert_eq!(
        capacity_bytes_from(Some(OsString::from("4096"))).unwrap(),
        4096
    );
}

// Host identity 위치가 현재 작업 디렉터리에 따라 달라지면 같은 사용자도 다른
// Host로 보이므로, 플랫폼 상태 환경변수의 상대 경로를 거부하는지 검증합니다.
#[test]
fn platform_state_root_rejects_relative_environment_paths() {
    #[cfg(target_os = "macos")]
    let result = platform_state_root_from(None, Some(OsString::from("relative-home")));
    #[cfg(not(target_os = "macos"))]
    let result = platform_state_root_from(
        Some(OsString::from("relative-state")),
        Some(OsString::from("/tmp/home")),
    );

    let error = result.expect_err("a relative platform state root must be rejected");
    assert!(error.to_string().contains("path is not absolute"));
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

#[cfg(not(target_os = "macos"))]
// Linux에서는 XDG state 위치가 HOME fallback보다 우선해 다른 XDG-aware CLI와
// 동일한 사용자 상태 디렉터리 규칙을 지킨다.
#[test]
fn linux_prefers_xdg_state_home() {
    let root = platform_state_root_from(
        Some(OsString::from("/tmp/xdg")),
        Some(OsString::from("/tmp/home")),
    )
    .unwrap();

    assert_eq!(root, PathBuf::from("/tmp/xdg/yo"));
}

#[cfg(target_os = "macos")]
// macOS에서는 별도 override가 없으면 사용자 Library의 Application Support 아래를
// 사용해 Session 파일이 일반 문서나 project 디렉터리에 섞이지 않게 한다.
#[test]
fn macos_uses_application_support() {
    let root = platform_state_root_from(None, Some(OsString::from("/tmp/home"))).unwrap();

    assert_eq!(
        root,
        PathBuf::from("/tmp/home/Library/Application Support/yo")
    );
}
