use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Barrier},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use uuid::{Uuid, Variant, Version};

use super::{HostWorkspacePath, LocalWorkspaceHostIdentity, WorkspaceHostId};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is after the Unix epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "yo-workspace-host-{}-{name}-{nonce}",
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

fn create_user_only_directory(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

// 새 Workspace Host ID는 머신 정보가 아니라 OS entropy만 사용한 RFC UUIDv4이며,
// 공개 문자열을 다시 읽어도 동일한 opaque identity를 얻는지 검증합니다.
#[test]
fn generated_workspace_host_identity_is_a_round_trippable_uuidv4() {
    let id = WorkspaceHostId::new().expect("the OS provides identity entropy");

    assert_eq!(id.as_uuid().get_version(), Some(Version::Random));
    assert_eq!(id.as_uuid().get_variant(), Variant::RFC4122);
    assert_eq!(id.to_string().parse::<WorkspaceHostId>().unwrap(), id);
    assert!(
        Uuid::now_v7()
            .to_string()
            .parse::<WorkspaceHostId>()
            .is_err()
    );
}

// 같은 workspace를 symlink 경로와 실제 경로로 열어도 Host가 canonical path 하나로
// 정규화해야 향후 기본 Session 목록이 같은 파일을 서로 다른 workspace로 나누지 않는다.
#[test]
fn local_workspace_normalization_collapses_a_symlink_alias() {
    let directory = TestDirectory::new("workspace-symlink");
    let workspace = directory.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let alias = directory.path().join("alias");
    symlink(&workspace, &alias).unwrap();

    assert_eq!(
        HostWorkspacePath::normalize_local(&workspace).unwrap(),
        HostWorkspacePath::normalize_local(&alias).unwrap()
    );
}

// 첫 open은 user-only 상태 디렉터리와 완결된 ID 파일을 만들고, 재실행은 새 ID를
// 만들지 않고 같은 값을 읽어 Session의 Host 소속이 안정적으로 유지되는지 검증합니다.
#[test]
fn creates_once_and_reopens_the_same_user_only_identity() {
    let directory = TestDirectory::new("stable");

    let first = LocalWorkspaceHostIdentity::open(directory.path()).unwrap();
    let second = LocalWorkspaceHostIdentity::open(directory.path()).unwrap();

    assert_eq!(second, first);
    assert_eq!(
        fs::read_to_string(directory.path().join("host-id")).unwrap(),
        format!("yo.workspace-host-id/v1 {}\n", first.id())
    );
    assert_eq!(
        fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(directory.path().join("host-id"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

// 여러 실행 흐름의 동시 opener가 모두 서로 다른 임시 후보를 만들더라도
// 완결된 final 파일 하나만 채택하고 모든 호출자가 같은 Host ID를 관찰하는지 검증합니다.
#[test]
fn concurrent_first_openers_converge_on_one_complete_identity() {
    let directory = TestDirectory::new("concurrent");
    let path = Arc::new(directory.path().to_owned());
    let barrier = Arc::new(Barrier::new(8));
    let workers = (0..8)
        .map(|_| {
            let path = Arc::clone(&path);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                LocalWorkspaceHostIdentity::open(path.as_path())
                    .unwrap()
                    .id()
            })
        })
        .collect::<Vec<_>>();
    let ids = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();

    assert!(ids.iter().all(|id| *id == ids[0]));
    assert_eq!(
        fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != "host-id")
            .count(),
        0
    );
}

// 0000, 0500, 0300처럼 서로 다른 owner-only 중간 mode를 만드는 제한적 umask에서도
// 동시 opener가 chmod 완료를 기다려 같은 ID와 정확한 0700/0600 경계로 수렴합니다.
#[test]
fn nested_creation_establishes_exact_modes_under_a_restrictive_umask() {
    const CHILD_PATH: &str = "YO_HOST_UMASK_TEST_PATH";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let path = Arc::new(PathBuf::from(path));
        let barrier = Arc::new(Barrier::new(8));
        let workers = (0..8)
            .map(|_| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    LocalWorkspaceHostIdentity::open(path.as_path())
                        .unwrap()
                        .id()
                })
            })
            .collect::<Vec<_>>();
        let ids = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert!(ids.iter().all(|id| *id == ids[0]));
        return;
    }

    let directory = TestDirectory::new("restrictive-umask");
    create_user_only_directory(directory.path());
    for mask in ["0777", "0277", "0477"] {
        let root = directory.path().join(mask).join("one/two/host");
        let status = Command::new("sh")
            .arg("-c")
            .arg("umask \"$1\"; shift; exec \"$@\"")
            .arg("sh")
            .arg(mask)
            .arg(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("host::tests::nested_creation_establishes_exact_modes_under_a_restrictive_umask")
            .arg("--nocapture")
            .env(CHILD_PATH, &root)
            .status()
            .unwrap();

        assert!(status.success(), "the {mask} umask child must converge");
        for path in [
            directory.path().join(mask),
            directory.path().join(mask).join("one"),
            directory.path().join(mask).join("one/two"),
            root.clone(),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        assert_eq!(
            fs::metadata(root.join("host-id"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

// 손상되거나 중간까지만 저장된 identity를 새 값으로 조용히 교체하면 기존 Session의
// Host 소속이 바뀌므로, 명시적인 오류를 반환하고 원본 bytes를 보존하는지 검증합니다.
#[test]
fn rejects_an_invalid_existing_identity_without_replacing_it() {
    let directory = TestDirectory::new("invalid");
    create_user_only_directory(directory.path());
    let path = directory.path().join("host-id");
    fs::write(&path, b"partial\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let error = LocalWorkspaceHostIdentity::open(directory.path())
        .expect_err("an invalid durable identity must not be replaced");

    assert!(error.to_string().contains("unsupported schema"));
    assert_eq!(fs::read(path).unwrap(), b"partial\n");
}

// 다른 사용자가 읽을 수 있는 identity 파일은 descriptor에 신뢰 가능한 per-user Host
// 경계를 제공하지 못하므로, permission을 자동 수정하지 않고 거부하는지 검증합니다.
#[test]
fn rejects_an_identity_file_with_non_user_only_permissions() {
    let directory = TestDirectory::new("permissions");
    create_user_only_directory(directory.path());
    let path = directory.path().join("host-id");
    let id = WorkspaceHostId::new().unwrap();
    fs::write(&path, format!("yo.workspace-host-id/v1 {id}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    let error = LocalWorkspaceHostIdentity::open(directory.path())
        .expect_err("a broadly readable identity must be rejected");

    assert!(error.to_string().contains("not 600"));
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

// 이미 존재하던 상태 루트가 다른 사용자에게 열려 있으면 이를 자동으로 고치는 순간
// 손상된 경계를 정상으로 오인할 수 있으므로, 권한과 내용을 바꾸지 않고 거부합니다.
#[test]
fn rejects_an_existing_permission_unsafe_state_root_without_repairing_it() {
    let directory = TestDirectory::new("unsafe-root");
    fs::create_dir_all(directory.path()).unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o777)).unwrap();

    let error = LocalWorkspaceHostIdentity::open(directory.path())
        .expect_err("an existing broadly accessible state root must be rejected");

    assert!(error.to_string().contains("not 700"));
    assert_eq!(
        fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
        0o777
    );
    assert!(!directory.path().join("host-id").exists());
}

// host 루트가 0700이어도 바로 위 상태 디렉터리를 다른 사용자가 쓸 수 있으면
// host 전체를 바꿔치기할 수 있으므로, 상위 권한을 고치지 않고 시작을 거부합니다.
#[test]
fn rejects_a_state_root_below_a_world_writable_parent() {
    let directory = TestDirectory::new("unsafe-parent");
    create_user_only_directory(directory.path());
    let parent = directory.path().join("state");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
    let root = parent.join("host");
    create_user_only_directory(&root);

    let error = LocalWorkspaceHostIdentity::open(&root)
        .expect_err("a replaceable state root must not establish an identity");

    assert!(error.to_string().contains("writable by other users"));
    assert_eq!(
        fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
        0o777
    );
    assert!(!root.join("host-id").exists());
}

// unsafe한 원래 경로의 중간 symlink가 안전한 0700 target을 가리켜도 canonicalize가
// 원래 부모를 숨기기 전에 거부해, target 밖으로 Host identity가 우회 생성되지 않습니다.
#[test]
fn rejects_an_unsafe_original_parent_before_following_an_ancestor_symlink() {
    let directory = TestDirectory::new("unsafe-ancestor-symlink");
    create_user_only_directory(directory.path());
    let unsafe_parent = directory.path().join("unsafe");
    fs::create_dir(&unsafe_parent).unwrap();
    fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o777)).unwrap();
    let target = directory.path().join("target");
    create_user_only_directory(&target);
    let target_root = target.join("host");
    create_user_only_directory(&target_root);
    symlink(&target, unsafe_parent.join("redirect")).unwrap();

    let error = LocalWorkspaceHostIdentity::open(unsafe_parent.join("redirect/host"))
        .expect_err("canonicalization must not hide an unsafe original parent");

    assert!(error.to_string().contains("writable by other users"));
    assert!(!target_root.join("host-id").exists());
}

// 현재 사용자가 소유한 sticky 부모는 다른 사용자가 그 안의 user-owned host 항목을
// 바꿀 수 없으므로, system-owned /tmp와 같은 보호를 인정해 정상적으로 다시 엽니다.
#[test]
fn accepts_a_current_user_owned_sticky_parent() {
    let directory = TestDirectory::new("user-sticky-parent");
    create_user_only_directory(directory.path());
    let parent = directory.path().join("sticky");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o1777)).unwrap();
    let root = parent.join("host");

    let first = LocalWorkspaceHostIdentity::open(&root).unwrap();
    let second = LocalWorkspaceHostIdentity::open(&root).unwrap();

    assert_eq!(second, first);
}

// 상태 루트 자체가 symlink이면 canonicalize 전에 외부 디렉터리로 경계가 바뀔 수
// 있으므로, 링크 대상 디렉터리에 host-id를 만들지 않고 거부하는지 검증합니다.
#[test]
fn rejects_a_symbolic_link_at_the_state_root() {
    let directory = TestDirectory::new("root-symlink");
    let target = directory.path().join("target");
    create_user_only_directory(&target);
    let root = directory.path().join("root");
    symlink(&target, &root).unwrap();

    let error = LocalWorkspaceHostIdentity::open(&root)
        .expect_err("the state root must not follow a symbolic link");

    assert!(error.to_string().contains("must not be a symbolic link"));
    assert!(!target.join("host-id").exists());
}

// owner만 접근할 수 있어도 계약보다 좁은 mode는 다음 실행의 reopen을 깨뜨릴 수
// 있으므로, 정확한 0600이 아닌 기존 identity 파일을 자동 수정 없이 거부합니다.
#[test]
fn rejects_an_identity_file_without_exact_permissions() {
    let directory = TestDirectory::new("exact-permissions");
    create_user_only_directory(directory.path());
    let path = directory.path().join("host-id");
    let id = WorkspaceHostId::new().unwrap();
    fs::write(&path, format!("yo.workspace-host-id/v1 {id}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();

    let error = LocalWorkspaceHostIdentity::open(directory.path())
        .expect_err("the identity file must use exactly mode 0600");

    assert!(error.to_string().contains("not 600"));
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o400
    );
}

// identity 경로의 symlink를 따라가면 외부 파일이 Host authority가 될 수 있으므로,
// 링크 대상의 내용이 유효해도 경계에서 거부하는지 검증합니다.
#[test]
fn rejects_a_symbolic_link_at_the_identity_path() {
    let directory = TestDirectory::new("symlink");
    create_user_only_directory(directory.path());
    let target = directory.path().join("target");
    fs::write(
        &target,
        format!(
            "yo.workspace-host-id/v1 {}\n",
            WorkspaceHostId::new().unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, directory.path().join("host-id")).unwrap();

    let error = LocalWorkspaceHostIdentity::open(directory.path())
        .expect_err("the identity path must not follow a symlink");

    assert!(error.to_string().contains("symbolic links"));
}
