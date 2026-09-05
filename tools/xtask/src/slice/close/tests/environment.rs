use std::process::Command;

use super::{CloseFixture, output};
use crate::{slice::close::apply, test_support};

const CHILD_MARKER: &str = "YO_XTASK_POISONED_GIT_CHILD";

// hook가 전달할 수 있는 repository-local Git 환경을 decoy 저장소로 오염시켜도
// child fixture의 commit과 ref가 decoy나 실제 후보 저장소를 수정하지 않는다.
#[test]
fn git_helpers_ignore_poisoned_hook_environment() {
    let decoy = test_support::TestRepository::new("slice-close-env-decoy");
    decoy.write("decoy.txt", "unchanged\n");
    decoy.git(["add", "decoy.txt"]);
    decoy.git(["commit", "--quiet", "-m", "test: decoy"]);
    let head = output(&decoy.path, &["rev-parse", "HEAD"]);
    let git_dir = decoy.path.join(".git");

    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "slice::close::tests::environment::poisoned_hook_child",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env("GIT_DIR", &git_dir)
        .env("GIT_COMMON_DIR", &git_dir)
        .env("GIT_WORK_TREE", &decoy.path)
        .env("GIT_INDEX_FILE", git_dir.join("index"))
        .env("GIT_OBJECT_DIRECTORY", git_dir.join("objects"))
        .env("GIT_SHALLOW_FILE", git_dir.join("shallow"))
        .env("GIT_CONFIG", git_dir.join("poisoned-config"))
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_0", git_dir.join("poisoned-hooks"))
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(output(&decoy.path, &["rev-parse", "HEAD"]), head);
    assert!(output(&decoy.path, &["status", "--porcelain"]).is_empty());
}

// parent test가 주입한 hook 환경 안에서 실제 fixture 생성·commit·plan을 수행해
// 모든 custom Git 경로가 중앙 격리 helper를 통과하는지 관찰한다.
#[test]
fn poisoned_hook_child() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    assert_eq!(plan.slice_ref, "refs/heads/slice/direct/sample");
}

#[cfg(unix)]
// common Git directory의 lock 자리가 FIFO여도 nonblocking open 뒤 regular-file
// 검사에서 즉시 거절하여 파괴적 apply가 writer를 기다리며 멈추지 않는다.
#[test]
fn rejects_fifo_cleanup_lock_without_blocking() {
    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    fixture.write_plan(&plan);
    let lock = fixture.repository.path.join(".git/yo-slice-close.lock");
    assert!(
        Command::new("mkfifo")
            .arg(&lock)
            .status()
            .unwrap()
            .success()
    );

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("Slice close lock"));
    assert!(fixture.slice_worktree.exists());
    std::fs::remove_file(lock).unwrap();
}

#[cfg(unix)]
// cleanup lock이 symlink면 외부 파일을 따라가 서로 다른 잠금을 잡을 수 있으므로
// NOFOLLOW open에서 거절하고 worktree와 Slice ref를 그대로 보존한다.
#[test]
fn rejects_symlink_cleanup_lock() {
    use std::os::unix::fs::symlink;

    let fixture = CloseFixture::new();
    let plan = fixture.plan();
    fixture.write_plan(&plan);
    let target = test_support::unique_path("slice-close-lock-target");
    let lock = fixture.repository.path.join(".git/yo-slice-close.lock");
    std::fs::write(&target, b"").unwrap();
    symlink(&target, &lock).unwrap();

    let error = apply(&fixture.repository.path, &fixture.plan_path).unwrap_err();

    assert!(error.contains("cannot open Slice close lock"));
    assert!(fixture.slice_worktree.exists());
    std::fs::remove_file(lock).unwrap();
    std::fs::remove_file(target).unwrap();
}
