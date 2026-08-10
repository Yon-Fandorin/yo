use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

use super::{
    super::{LocalWorkspaceHostIdentity, WorkspaceHostId},
    support::{TestDirectory, create_user_only_directory},
};

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
