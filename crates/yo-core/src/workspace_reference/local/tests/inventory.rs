use std::{
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

use super::{
    super::{build_inventory, discover_entries, git_command},
    support::{TempFixture, host_id},
};
use crate::WorkspaceReferenceKind;

// 실제 Git 저장소에서 nested .gitignore와 repository exclude를 적용하면서
// 숨김 파일과 파일을 가진 디렉터리는 후보로 남기고 Git 내부는 노출하지 않는다.
#[test]
fn inventory_uses_the_effective_repository_ignore_stack() {
    let fixture = TempFixture::new("effective-git-ignore");
    let root = fixture.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join("src/nested")).unwrap();
    fs::create_dir(root.join("ignored")).unwrap();
    assert!(
        git_command(root)
            .arg("init")
            .arg("-q")
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    fs::write(root.join(".git/info/exclude"), "local-only.txt\n").unwrap();
    fs::write(root.join(".hidden"), "visible").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(root.join("src/nested/.gitignore"), "skip.md\n").unwrap();
    fs::write(root.join("src/nested/keep.md"), "keep\n").unwrap();
    fs::write(root.join("src/nested/skip.md"), "skip\n").unwrap();
    fs::write(root.join("ignored/private.txt"), "private\n").unwrap();
    fs::write(root.join("ignored/tracked.txt"), "tracked\n").unwrap();
    fs::write(root.join("local-only.txt"), "local\n").unwrap();
    symlink("src", root.join("linked-src")).unwrap();
    symlink("/tmp", root.join("outside-link")).unwrap();
    assert!(
        git_command(root)
            .args(["add", "-f", "ignored/tracked.txt"])
            .status()
            .unwrap()
            .success()
    );

    let inventory = build_inventory(root, host_id()).unwrap();
    let paths = inventory
        .entries
        .iter()
        .map(|entry| entry.reference().relative_path())
        .collect::<Vec<_>>();
    assert!(paths.contains(&".hidden"));
    assert!(paths.contains(&"src"));
    assert!(paths.contains(&"src/nested/keep.md"));
    assert!(!paths.iter().any(|path| path.starts_with(".git/")));
    assert!(paths.contains(&"ignored"));
    assert!(paths.contains(&"ignored/tracked.txt"));
    assert!(!paths.contains(&"ignored/private.txt"));
    assert!(!paths.contains(&"src/nested/skip.md"));
    assert!(!paths.contains(&"local-only.txt"));
    assert!(!paths.contains(&"linked-src"));
    assert!(!paths.contains(&"outside-link"));
}

// 같은 fixture를 plain 상태와 Git 상태로 차례로 검사해, Git root가 확인된 경우에만 ignore 정책을
// 적용하고 일반 디렉터리에서는 .gitignore를 잘못 적용하지 않는지 확인한다.
#[test]
fn inventory_selects_git_ignore_policy_only_for_git_workspaces() {
    let fixture = TempFixture::new("ignore-policy");
    fs::create_dir(fixture.path().join("ignored")).unwrap();
    fs::write(fixture.path().join(".gitignore"), "ignored/\n").unwrap();
    fs::write(fixture.path().join("ignored/plain.txt"), "plain\n").unwrap();

    let plain_inventory = build_inventory(fixture.path(), host_id()).unwrap();
    assert!(
        plain_inventory
            .entries
            .iter()
            .any(|entry| entry.reference().relative_path() == "ignored/plain.txt")
    );

    assert!(
        git_command(fixture.path())
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    let git_inventory = build_inventory(fixture.path(), host_id()).unwrap();
    assert!(
        !git_inventory
            .entries
            .iter()
            .any(|entry| entry.reference().relative_path() == "ignored/plain.txt")
    );
    assert!(
        git_inventory
            .entries
            .iter()
            .any(|entry| entry.reference().relative_path() == ".gitignore")
    );
}

// 같은 host의 반복 inventory가 안정적인 reference를 만들고 다른 host에서는 root/item identity를
// 유지하면서 provenance만 달라지는지, directory symlink가 후보에서 빠지는지 확인한다.
#[test]
fn inventory_assigns_stable_identities_and_excludes_directory_symlinks() {
    let fixture = TempFixture::new("inventory-identity");
    fs::create_dir(fixture.path().join("src")).unwrap();
    fs::write(fixture.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    symlink("src", fixture.path().join("src-link")).unwrap();
    let canonical_root = fs::canonicalize(fixture.path()).unwrap();
    let same_host = host_id();
    let different_host = "20000000-0000-4000-8000-000000000002".parse().unwrap();
    let inventory = build_inventory(&canonical_root, same_host).unwrap();
    let repeated_inventory = build_inventory(&canonical_root, same_host).unwrap();
    let different_host_inventory = build_inventory(&canonical_root, different_host).unwrap();

    let paths = inventory
        .entries
        .iter()
        .map(|entry| entry.reference().relative_path())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"src"));
    assert!(paths.contains(&"src/main.rs"));
    assert!(!paths.contains(&"src-link"));
    assert_eq!(inventory.entries, repeated_inventory.entries);
    assert_eq!(
        inventory.entries.len(),
        different_host_inventory.entries.len()
    );
    for (entry, different_host_entry) in inventory
        .entries
        .iter()
        .zip(&different_host_inventory.entries)
    {
        assert_eq!(
            entry.reference().relative_path(),
            different_host_entry.reference().relative_path()
        );
        assert_eq!(
            entry.reference().kind(),
            different_host_entry.reference().kind()
        );
        assert_eq!(
            entry.reference().identity(),
            different_host_entry.reference().identity()
        );
        assert_eq!(
            entry.reference().root_identity(),
            different_host_entry.reference().root_identity()
        );
        assert_ne!(
            entry.reference().execution_environment_identity(),
            different_host_entry
                .reference()
                .execution_environment_identity()
        );
        assert_ne!(
            entry.reference().workspace_identity(),
            different_host_entry.reference().workspace_identity()
        );
    }
    let directory = inventory
        .entries
        .iter()
        .find(|entry| entry.reference().relative_path() == "src")
        .unwrap();
    assert_eq!(
        directory.reference().kind(),
        WorkspaceReferenceKind::Directory
    );
    let file = inventory
        .entries
        .iter()
        .find(|entry| entry.reference().relative_path() == "src/main.rs")
        .unwrap();
    assert_eq!(file.reference().kind(), WorkspaceReferenceKind::File);
}

// host filesystem이 UTF-8이 아닌 entry를 허용하면 inventory가 lossy 후보 없이
// Incomplete를 보고하고, 허용하지 않으면 fixture 생성의 EILSEQ만 구분한다.
#[test]
fn inventory_marks_non_utf8_entries_incomplete_without_lossy_candidates() {
    let fixture = TempFixture::new("inventory-non-utf8");
    let invalid_name = OsString::from_vec(b"bad-\xff".to_vec());
    if let Err(error) = fs::write(fixture.path().join(&invalid_name), "bad\n") {
        assert_eq!(
            error.raw_os_error(),
            Some(libc::EILSEQ),
            "creating the non-UTF-8 fixture entry under {} failed unexpectedly: {error}",
            fixture.path().display()
        );
        return;
    }
    fs::write(fixture.path().join("normal.txt"), "normal\n").unwrap();

    let inventory = build_inventory(fixture.path(), host_id()).unwrap();
    match &inventory.status {
        crate::WorkspaceReferenceSearchStatus::Incomplete(reason) => {
            assert!(!reason.trim().is_empty());
        },
        status => panic!("expected incomplete inventory, got {status:?}"),
    }
    let paths = inventory
        .entries
        .iter()
        .map(|entry| entry.reference().relative_path())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"normal.txt"));
    assert!(!paths.iter().any(|path| path.contains('\u{fffd}')));
}

// Git 저장소가 아닌 일반 작업 디렉터리도 파일과 디렉터리를 같은 후보 계약으로 제공한다.
#[test]
fn discovery_supports_a_plain_non_git_workspace() {
    let fixture = TempFixture::new("plain-workspace");
    let root = fixture.path();
    fs::create_dir(root.join("notes")).unwrap();
    fs::create_dir(root.join("notes/drafts")).unwrap();
    fs::write(root.join("notes/drafts/plan.md"), "plan\n").unwrap();

    let (entries, incomplete) = discover_entries(root, false).unwrap();
    assert!(!incomplete);
    let paths = entries
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"notes"));
    assert!(paths.contains(&"notes/drafts"));
    assert!(paths.contains(&"notes/drafts/plan.md"));
}
