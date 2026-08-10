use super::super::*;

// capture와 최종 publish에서 쓰는 branch guard가 같은 commit을 가리키더라도
// 다른 branch identity는 거부함을 확인한다.
#[test]
fn expected_branch_identity_is_not_just_a_commit_check() {
    let repository = crate::test_support::TestRepository::new("review-delta-branch");
    repository.write("file.txt", "base\n");
    repository.git(["add", "file.txt"]);
    repository.git(["commit", "--quiet", "-m", "base"]);
    repository.git(["switch", "-c", "slice/direct/review-delta"]);
    require_expected_branch(&repository.path, "refs/heads/develop", "review-delta").unwrap();
    repository.git(["switch", "-c", "unrelated"]);
    assert!(
        require_expected_branch(&repository.path, "refs/heads/develop", "review-delta",)
            .unwrap_err()
            .contains("expected refs/heads/slice/direct/review-delta")
    );
}

// 동일한 게시 artifact의 여러 경로 표기를 하나의 repository-relative identity로
// 정규화해 alias가 별도 manifest identity를 만들지 않음을 확인한다.
#[test]
fn published_artifact_paths_are_canonicalized_before_identity_capture() {
    let root = crate::test_support::unique_path("review-delta-canonical-path");
    std::fs::create_dir_all(root.join("store")).unwrap();
    std::fs::write(root.join("store/manifest.json"), b"manifest\n").unwrap();
    let direct = capture_published(
        &root,
        &root.join("store/manifest.json"),
        "manifest",
        MAX_INPUT_BYTES,
    )
    .unwrap();
    let dotted = capture_published(
        &root,
        &root.join("store/../store/manifest.json"),
        "manifest",
        MAX_INPUT_BYTES,
    )
    .unwrap();
    assert_eq!(direct.path, "store/manifest.json");
    assert_eq!(direct.path, dotted.path);
    assert_eq!(direct.hash, dotted.hash);
    std::fs::remove_dir_all(root).unwrap();
}
