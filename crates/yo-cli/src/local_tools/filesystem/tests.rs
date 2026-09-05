use std::{
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    sync::atomic::AtomicBool,
};

use yo_core::ToolExecutionHost;

use super::{
    super::tests::{TestDirectory, finish, request},
    LocalToolHost,
    descriptor::open_regular_file,
    list::list_files,
    path,
    path::path_components,
    read::read_file,
};
use crate::local_tools::registry::{LocalToolRegistryRevision, registry};

// legacy read_file은 workspace 안의 일반 파일을 읽되 credential path 실패를 고정된
// execution result로 닫고, 잘못된 상위 경로는 worker 시작 전 거절합니다.
#[test]
fn reads_workspace_files_but_denies_the_credential_file() {
    let directory = TestDirectory::new();
    let source = directory.0.join("source.txt");
    let credential = directory.0.join("credentials.yaml");
    fs::write(&source, "hello").unwrap();
    let mut file = fs::File::create(&credential).unwrap();
    file.write_all(b"secret").unwrap();
    fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();
    let registry = registry(LocalToolRegistryRevision::LegacyReadFile).unwrap();
    let mut host = LocalToolHost::new(&directory.0, &credential).unwrap();

    let mut execution = host
        .start(request(&registry, "read_file", r#"{"path":"source.txt"}"#))
        .unwrap();
    assert_eq!(finish(execution.as_mut()).output(), "hello");
    let mut denied = host
        .start(request(
            &registry,
            "read_file",
            r#"{"path":"credentials.yaml"}"#,
        ))
        .unwrap();
    let denied = finish(denied.as_mut());
    assert_eq!(denied.outcome(), yo_core::ToolExecutionOutcome::Failed);
    assert_eq!(denied.output(), "tool execution failed");
    assert!(
        host.start(request(&registry, "read_file", r#"{"path":"../outside"}"#))
            .is_err()
    );
}

// 요청 시 연 workspace-relative handle을 작업 thread까지 넘기므로 이후 경로가
// workspace 밖 symlink로 교체되어도 read/list 대상이 바뀌지 않는다.
#[test]
fn opened_workspace_handles_resist_later_symlink_replacement() {
    let workspace = TestDirectory::new();
    let outside = TestDirectory::new();
    let source = workspace.0.join("source.txt");
    let original_source = workspace.0.join("source-original.txt");
    let outside_source = outside.0.join("outside.txt");
    fs::write(&source, "inside").unwrap();
    fs::write(&outside_source, "outside").unwrap();
    let listed = workspace.0.join("listed");
    let original_listed = workspace.0.join("listed-original");
    fs::create_dir(&listed).unwrap();
    fs::write(listed.join("inside.txt"), "inside").unwrap();
    let outside_listed = outside.0.join("listed");
    fs::create_dir(&outside_listed).unwrap();
    fs::write(outside_listed.join("outside.txt"), "outside").unwrap();
    let host = LocalToolHost::new(&workspace.0, &workspace.0.join("credentials.yaml")).unwrap();

    let components = path_components("source.txt").unwrap();
    let file = open_regular_file(
        &host.workspace_directory,
        &components,
        host.denied_credential,
    )
    .unwrap();
    let (directory, relative) = host.open_directory("listed").unwrap();
    fs::rename(&source, &original_source).unwrap();
    symlink(&outside_source, &source).unwrap();
    fs::rename(&listed, &original_listed).unwrap();
    symlink(&outside_listed, &listed).unwrap();

    assert_eq!(
        read_file(file, 1024, &AtomicBool::new(false)).output(),
        "inside"
    );
    let listing = list_files(directory, relative, 1024, &AtomicBool::new(false));
    assert!(listing.output().contains("listed/inside.txt"));
    assert!(!listing.output().contains("outside.txt"));
}

// list_files 경로는 file 경로와 같은 byte/control/traversal 경계를 사용하지만 `.`의
// 정규화 결과인 workspace root는 허용해, root의 immediate child를 안전하게 나열합니다.
#[test]
fn list_path_admission_allows_root_and_rejects_ambiguous_inputs_before_open() {
    let workspace = TestDirectory::new();
    fs::write(workspace.0.join("root.txt"), "root").unwrap();
    fs::create_dir(workspace.0.join("nested")).unwrap();
    fs::write(workspace.0.join("nested/child.txt"), "child").unwrap();
    let host = LocalToolHost::new(&workspace.0, &workspace.0.join("credentials.yaml")).unwrap();
    let (directory, relative) = host.open_directory("./.").unwrap();
    let result = list_files(directory, relative, 1024, &AtomicBool::new(false));
    assert_eq!(result.output(), "nested/\nroot.txt\n");
    let (directory, relative) = host.open_directory("./nested/./").unwrap();
    let nested = list_files(directory, relative, 1024, &AtomicBool::new(false));
    assert_eq!(nested.output(), "nested/child.txt\n");
    assert!(path::basic_path(".").is_err());

    for rejected in ["", "../outside", "/absolute", "bad\nname"] {
        assert!(
            host.open_directory(rejected).is_err(),
            "accepted {rejected:?}"
        );
    }
    assert!(host.open_directory(&"a".repeat(1_025)).is_err());
}

// 선택 directory의 child는 fstatat만 한 번 호출해 regular와 directory를 표시하고,
// nested content, symlink, FIFO, `.git`은 열거나 재귀 방문하지 않습니다.
#[test]
fn lists_only_immediate_children_without_opening_them() {
    let workspace = TestDirectory::new();
    let listed = workspace.0.join("listed");
    let nested = listed.join("nested");
    let visible = listed.join("visible.txt");
    fs::create_dir(&listed).unwrap();
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("hidden.txt"), "hidden").unwrap();
    fs::write(&visible, "visible").unwrap();
    fs::create_dir(listed.join(".git")).unwrap();
    symlink("visible.txt", listed.join("link")).unwrap();
    nix::unistd::mkfifo(&listed.join("pipe"), nix::sys::stat::Mode::S_IRUSR).unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o000)).unwrap();
    fs::set_permissions(&visible, fs::Permissions::from_mode(0o000)).unwrap();

    let host = LocalToolHost::new(&workspace.0, &workspace.0.join("credentials.yaml")).unwrap();
    let (directory, relative) = host.open_directory("listed").unwrap();
    let result = list_files(directory, relative, 1024, &AtomicBool::new(false));

    fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&visible, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(result.outcome(), yo_core::ToolExecutionOutcome::Completed);
    assert_eq!(result.output(), "listed/nested/\nlisted/visible.txt\n");
    assert!(!result.truncated());
}
