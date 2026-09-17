use super::*;

// 실행 전 output directory가 비어 있어야 claim과 결과가 이전 attempt의 파일 위에
// 겹치지 않고, 첫 파일이 생긴 순간 같은 경로의 재사용을 거부합니다.
#[test]
fn output_directory_must_be_empty_before_claim() {
    let repository = TestRepository::new("review-delivery-output");
    let output = repository.path.join("output");
    fs::create_dir(&output).unwrap();
    require_empty_directory(&output).unwrap();
    fs::write(output.join("claim.json"), b"claimed").unwrap();
    assert!(
        require_empty_directory(&output)
            .unwrap_err()
            .contains("must be empty")
    );
}

// 로컬 준비 단계가 exact output child를 만들고 쓰기 probe를 회수하므로, 존재하지
// 않는 디렉터리가 Provider request 전의 delivery 실패로 잘못 계산되지 않습니다.
#[test]
fn output_directory_preparation_creates_and_checks_exact_child() {
    let repository = TestRepository::new("review-delivery-output-prepare");
    let coordination = repository.path.join("coordination");
    fs::create_dir(&coordination).unwrap();
    let output = coordination.join("attempt-1");

    let prepared = prepare_output_directory_at(&coordination, &output).unwrap();
    assert_eq!(prepared, fs::canonicalize(&output).unwrap());
    assert!(fs::read_dir(&prepared).unwrap().next().is_none());
    prepare_output_directory_at(&coordination, &output).unwrap();
}

// 이전 attempt 파일이 남은 경로는 준비 단계에서 거부해 새 claim이나 외부 요청이
// 기존 결과와 섞이지 않도록 합니다.
#[test]
fn output_directory_preparation_rejects_nonempty_directory() {
    let repository = TestRepository::new("review-delivery-output-nonempty");
    let coordination = repository.path.join("coordination");
    let output = coordination.join("attempt-1");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("claim.json"), b"claimed").unwrap();

    assert!(
        prepare_output_directory_at(&coordination, &output)
            .unwrap_err()
            .contains("must be empty")
    );
}

#[cfg(unix)]
// Slice 안을 가리키더라도 최종 symlink는 실제 output 소유권을 흐리므로 claim 전에
// 거부해 경로 교체나 alias를 통한 결과 덮어쓰기를 막습니다.
#[test]
fn output_directory_preparation_rejects_final_symlink() {
    let repository = TestRepository::new("review-delivery-output-symlink");
    let coordination = repository.path.join("coordination");
    let target = coordination.join("target");
    let output = coordination.join("attempt-1");
    fs::create_dir_all(&target).unwrap();
    symlink(&target, &output).unwrap();

    assert!(
        prepare_output_directory_at(&coordination, &output)
            .unwrap_err()
            .contains("real directory")
    );
}

// develop build의 exactness 검사에는 tracked 변경뿐 아니라 build resolution을 바꿀 수
// 있는 untracked 파일도 포함되어야 하며, 생긴 즉시 claim 전에 거부합니다.
#[test]
fn integration_state_rejects_untracked_files() {
    let repository = TestRepository::new("review-delivery-integration-clean");
    repository.write("tracked.txt", "tracked\n");
    repository.git(["add", "tracked.txt"]);
    repository.git(["commit", "--quiet", "-m", "test: base"]);
    let head = git::trusted_output_in(&repository.path, &["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_owned();
    require_integration_state(&repository.path, &head).unwrap();

    repository.write("untracked.txt", "can affect resolution\n");
    assert!(
        require_integration_state(&repository.path, &head)
            .unwrap_err()
            .contains("must be clean")
    );
}
