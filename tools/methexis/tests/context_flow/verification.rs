use std::{fs, os::unix::fs::symlink, path::Path};

use super::support::{active_repository, direct_request, resolve, verify, verify_failure};

// 깊은 검증은 저장된 산출물을 재사용해 성공하는 것이 아니라 요청과 현재 authority로 독립 재현한
// 뒤 같은 BuildId와 두 파일을 확인한다. 성공 결과가 현재 commit과 hash를 반환하면서 관리 build의
// 파일 집합과 bytes를 바꾸지 않는지 함께 확인한다.
#[test]
fn verifier_reproduces_a_build_and_leaves_the_managed_artifacts_unchanged() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    let created = resolve(&repository, &request);
    let build_id = created["build_id"].as_str().unwrap();
    let build = repository
        .path
        .join(created["context"]["path"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_owned();
    let before = snapshot(&build);

    let verified = verify(&repository, &request, build_id);

    assert_eq!(
        verified["schema"],
        "methexis.context-verification-result/v1alpha1"
    );
    assert_eq!(verified["operation"], "verify_context_build");
    assert_eq!(verified["status"], "verified");
    assert_eq!(verified["build_id"], created["build_id"]);
    assert_eq!(verified["trusted_commit"], created["trusted_commit"]);
    assert_eq!(verified["context"]["hash"], created["context"]["hash"]);
    assert_eq!(verified["manifest"]["hash"], created["manifest"]["hash"]);
    assert_eq!(snapshot(&build), before);
}

// 기대 BuildId가 독립 컴파일 결과와 다르면 그 이름의 관리 경로가 symlink여도 읽지 않아야 한다.
// 따라서 저장 경로 오류가 아니라 identity mismatch로 끝나야 하며, target consult가 최종 비교 뒤로
// 제한된다는 순서 계약을 구별한다.
#[test]
fn derived_identity_mismatch_fails_before_the_named_build_is_consulted() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    resolve(&repository, &request);
    let other_id = format!("sha256:{}", "f".repeat(64));
    let other = repository
        .path
        .join(".local-exclude/methexis/builds")
        .join(other_id.strip_prefix("sha256:").unwrap());
    let outside = repository.path.join("outside-build");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, &other).unwrap();

    let failure = verify_failure(&repository, &request, &other_id);

    assert_eq!(failure["operation"], "verify_context_build");
    assert_eq!(failure["error"]["code"], "context_build_identity_mismatch");
}

// BuildId 인자는 경로 조각으로 쓰이기 전에 canonical lowercase sha256 문법을 만족해야 한다.
// 잘못된 값은 요청을 성공시키거나 저장 경로를 만들지 않고 구조화된 검증 오류로 끝난다.
#[test]
fn invalid_build_id_syntax_fails_closed() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);

    let failure = verify_failure(&repository, &request, "sha256:NOT-A-BUILD-ID");

    assert_eq!(failure["operation"], "verify_context_build");
    assert_eq!(failure["error"]["code"], "invalid_context_build_id");
}

// 관리 build가 사라지거나 bytes, 닫힌 파일 집합, 파일 종류, 디렉터리 symlink 경계를 벗어나면
// 어느 경우도 검증 성공으로 축약하지 않는다. 각 변조 뒤 원래 상태를 복구해 같은 기준 build에
// 대한 독립 실패 경계를 확인한다.
#[test]
fn missing_corrupt_extra_nonregular_and_symlinked_builds_all_fail() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    let created = resolve(&repository, &request);
    let build_id = created["build_id"].as_str().unwrap();
    let build = repository
        .path
        .join(created["context"]["path"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_owned();
    let saved = build.with_extension("saved");

    fs::rename(&build, &saved).unwrap();
    assert_eq!(
        verify_failure(&repository, &request, build_id)["error"]["code"],
        "context_build_missing"
    );
    fs::rename(&saved, &build).unwrap();

    let context = build.join("context.md");
    let context_bytes = fs::read(&context).unwrap();
    fs::write(&context, b"corrupt\n").unwrap();
    assert_verification_failed(&repository, &request, build_id);
    fs::write(&context, &context_bytes).unwrap();

    fs::write(build.join("extra"), b"unexpected\n").unwrap();
    assert_verification_failed(&repository, &request, build_id);
    fs::remove_file(build.join("extra")).unwrap();

    fs::remove_file(&context).unwrap();
    fs::create_dir(&context).unwrap();
    assert_verification_failed(&repository, &request, build_id);
    fs::remove_dir(&context).unwrap();
    fs::write(&context, &context_bytes).unwrap();

    fs::rename(&build, &saved).unwrap();
    symlink(&saved, &build).unwrap();
    let failure = verify_failure(&repository, &request, build_id);
    assert_eq!(failure["error"]["code"], "context_path_symlink");
    fs::remove_file(&build).unwrap();
    fs::rename(&saved, &build).unwrap();
}

fn assert_verification_failed(
    repository: &super::repository::GitRepository,
    request: &Path,
    build_id: &str,
) {
    assert_eq!(
        verify_failure(repository, request, build_id)["error"]["code"],
        "context_build_verification_failed"
    );
}

fn snapshot(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}
