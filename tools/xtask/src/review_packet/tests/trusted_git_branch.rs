use super::super::{
    capture::capture_authorities,
    trusted_git::{expected_slice_ref, trusted_ensure_clean},
};

#[cfg(unix)]
// immutable authority capture는 repository replacement ref를 무시해 recorded commit의
// 원래 blob을 읽고, executable이나 symlink mode를 regular authority로 받아들이지 않는다.
#[test]
fn trusted_git_capture_ignores_replacements_and_rejects_non_regular_modes() {
    use std::os::unix::fs::PermissionsExt;

    let repository = crate::test_support::unique_path("slice-review-trusted-git");
    std::fs::create_dir(&repository).unwrap();
    test_git(&repository, &["init", "--initial-branch=develop"]);
    test_git(
        &repository,
        &["config", "user.email", "fixture@example.invalid"],
    );
    test_git(&repository, &["config", "user.name", "Fixture"]);
    std::fs::write(repository.join("authority.md"), b"original\n").unwrap();
    test_git(&repository, &["add", "authority.md"]);
    test_git(&repository, &["commit", "-m", "original"]);
    let original = test_git(&repository, &["rev-parse", "HEAD"]);

    std::fs::write(repository.join("authority.md"), b"replacement\n").unwrap();
    test_git(&repository, &["commit", "-am", "replacement"]);
    let replacement = test_git(&repository, &["rev-parse", "HEAD"]);
    test_git(
        &repository,
        &["replace", original.trim(), replacement.trim()],
    );

    let captured =
        capture_authorities(&repository, original.trim(), &["authority.md".to_owned()]).unwrap();
    assert_eq!(captured[0].bytes, b"original\n");

    let mut permissions = std::fs::metadata(repository.join("authority.md"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(repository.join("authority.md"), permissions).unwrap();
    test_git(&repository, &["add", "authority.md"]);
    test_git(&repository, &["commit", "-m", "executable"]);
    let executable = test_git(&repository, &["rev-parse", "HEAD"]);
    let error = capture_authorities(&repository, executable.trim(), &["authority.md".to_owned()])
        .unwrap_err();
    assert!(error.contains("non-executable regular Git blob"));
    std::fs::remove_dir_all(repository).unwrap();
}

// 직접 Slice와 Wave Slice는 같은 계약 규칙에서 정확한 branch ref를 도출하고,
// 지원하지 않는 integration ref는 임의 branch 이름으로 해석하지 않는다.
#[test]
fn expected_slice_branch_supports_direct_and_wave_slices() {
    assert_eq!(
        expected_slice_ref("refs/heads/develop", "sample").unwrap(),
        "refs/heads/slice/direct/sample"
    );
    assert_eq!(
        expected_slice_ref("refs/heads/wave/runtime", "sample").unwrap(),
        "refs/heads/slice/runtime/sample"
    );
    assert!(expected_slice_ref("refs/heads/main", "sample").is_err());
    assert!(expected_slice_ref("refs/heads/wave/a/b", "sample").is_err());
}

#[cfg(unix)]
// PATH의 가짜 git이 clean status를 반환해도 trusted command는 고정된 Git을 사용해
// 실제 untracked file을 보고 dirty candidate를 거부한다.
#[test]
fn trusted_cleanliness_ignores_path_injected_git() {
    use std::os::unix::fs::PermissionsExt;

    let repository = crate::test_support::unique_path("slice-review-path-injection");
    let fake_bin = crate::test_support::unique_path("slice-review-fake-git");
    std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(&fake_bin).unwrap();
    test_git(&repository, &["init", "--initial-branch=develop"]);
    test_git(
        &repository,
        &["config", "user.email", "fixture@example.invalid"],
    );
    test_git(&repository, &["config", "user.name", "Fixture"]);
    std::fs::write(repository.join("tracked"), b"tracked\n").unwrap();
    test_git(&repository, &["add", "tracked"]);
    test_git(&repository, &["commit", "-m", "initial"]);
    std::fs::write(repository.join("untracked"), b"must be observed\n").unwrap();

    let fake_git = fake_bin.join("git");
    std::fs::write(&fake_git, b"#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = std::fs::metadata(&fake_git).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_git, permissions).unwrap();

    let output = crate::git::trusted_command_in(&repository)
        .env("PATH", &fake_bin)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!output.stdout.is_empty());
    assert!(trusted_ensure_clean(&repository, "candidate", "reviewing").is_err());

    std::fs::remove_dir_all(repository).unwrap();
    std::fs::remove_dir_all(fake_bin).unwrap();
}

#[cfg(unix)]
fn test_git(repository: &std::path::Path, arguments: &[&str]) -> String {
    let output = std::process::Command::new("/usr/bin/git")
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("LC_ALL", "C")
        .current_dir(repository)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
