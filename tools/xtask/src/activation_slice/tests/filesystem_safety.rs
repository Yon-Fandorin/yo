use super::{
    super::{prepare, run},
    support::Fixture,
};
use crate::test_support;

#[cfg(unix)]
// dangling symlink도 Path::exists의 false가 아니라 점유된 path conflict로
// 관찰하여 contract publication 전에 반복 불가능한 setup을 차단한다.
#[test]
fn dangling_worktree_symlink_is_a_structured_conflict() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("activation-dangling-worktree");
    let parent = fixture.repository.path.join(".local-exclude/worktrees");
    std::fs::create_dir_all(&parent).unwrap();
    symlink(parent.join("missing-target"), fixture.worktree()).unwrap();

    let encoded = run(&fixture.repository.path, &fixture.request).unwrap_err();
    let failure: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(failure["effects"]["contract"]["state"], "absent");
    assert_eq!(failure["effects"]["worktree"]["state"], "conflicting");
}

#[cfg(unix)]
// coordination ancestor가 symlink면 component별 NOFOLLOW directory 생성이
// 외부 target 아래에 helper directory를 만들기 전에 실패한다.
#[test]
fn rejects_a_symlinked_local_directory_without_external_creation() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("activation-directory-symlink");
    let external = test_support::unique_path("activation-external-directory");
    std::fs::create_dir(&external).unwrap();
    std::fs::write(
        fixture.repository.path.join(".git/info/exclude"),
        ".local-exclude\n.local-exclude/\n",
    )
    .unwrap();
    symlink(&external, fixture.repository.path.join(".local-exclude")).unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("without symlinks"), "{error}");
    assert!(!external.join("coordination").exists());
    std::fs::remove_dir(external).unwrap();
}

// develop worktree가 dirty면 어느 commit과 working state를 기준으로 삼을지
// 모호하므로 local coordination directory나 Slice ref를 만들기 전에 멈춘다.
#[test]
fn rejects_a_dirty_integration_worktree_before_any_effect() {
    let fixture = Fixture::new("activation-dirty");
    fixture.repository.write("untracked.txt", "dirty\n");

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("integration worktree must be clean"));
    assert!(!fixture.worktree().exists());
}

// helper는 direct activation 전용이므로 다른 named branch에서 실행해 그
// branch를 develop authority로 오인하거나 Wave 계약을 조용히 만들지 않는다.
#[test]
fn rejects_a_non_develop_integration_branch() {
    let fixture = Fixture::new("activation-wrong-branch");
    fixture.repository.git(["switch", "--quiet", "-c", "other"]);

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("must run from `refs/heads/develop`"));
    assert!(!fixture.worktree().exists());
}

// contract를 먼저 발행한 뒤 Git branch 형식 오류로 영구 실패하지 않도록
// request의 Slice 이름이 유효한 전체 ref를 만드는지 side effect 전에 확인한다.
#[test]
fn rejects_an_invalid_git_branch_name_before_any_effect() {
    let fixture = Fixture::new("activation-invalid-ref");
    let invalid_slice = "activation..invalid";
    std::fs::write(
        &fixture.request,
        format!(
            r#"{{
  "schema": "yo.activation-slice-request/v1",
  "slice": "{invalid_slice}",
  "owned_contracts": ["test.activation"]
}}
"#
        ),
    )
    .unwrap();

    let error = prepare(&fixture.repository.path, &fixture.request).unwrap_err();

    assert!(error.contains("does not form a valid Git branch"));
    assert!(
        !fixture
            .repository
            .path
            .join(".local-exclude/coordination")
            .join(invalid_slice)
            .exists()
    );
}

#[cfg(unix)]
// request가 symlink면 외부에서 바뀌는 입력을 따라가지 않아 exact setup
// identity가 invocation 도중 다른 bytes로 바뀌는 경로를 닫는다.
#[test]
fn rejects_a_symlink_request() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("activation-request-symlink");
    let link = test_support::unique_path("activation-request-link.json");
    symlink(&fixture.request, &link).unwrap();

    let error = prepare(&fixture.repository.path, &link).unwrap_err();

    assert!(error.contains("cannot open activation Slice request"));
    std::fs::remove_file(link).unwrap();
}
