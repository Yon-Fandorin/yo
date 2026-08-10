use std::{fs, os::unix::fs::symlink};

use uuid::{Uuid, Variant, Version};

use super::{
    super::{HostWorkspacePath, WorkspaceHostId},
    support::TestDirectory,
};

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

// 원격 Host가 보낸 workspace bytes는 로컬 UTF-8로 재해석하지 않고, 정상 Unicode는
// 읽기 쉽게 유지하면서 잘못된 byte와 제어문자만 명시적으로 escape해 identity를 보존합니다.
#[test]
fn workspace_display_is_lossless_and_terminal_safe() {
    let path =
        HostWorkspacePath::from_unix_bytes(b"/work/\xEA\xB0\x80\\name\n\xFF".to_vec()).unwrap();

    assert_eq!(path.to_string(), "/work/가\\\\name\\n\\xFF");
}
