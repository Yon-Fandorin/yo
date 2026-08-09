use std::{fs, os::unix::fs::symlink};

use serde_json::json;

use super::{
    repository::GitRepository,
    support::{
        active_repository, candidate_request, direct_request, raw_resolve, resolve, resolve_failure,
    },
};

// 사용자가 직접 지정한 anchor를 찾지 못하거나 필수 지식 묶음이 token 예산에 들어가지 않으면
// context를 만들 수 없다. 부분 context를 stdout에 내보내지 않고 각각의 원인을 구조화된 오류로
// 보고한다.
#[test]
fn unresolved_direct_anchor_and_required_over_budget_fail_without_stdout() {
    let repository = active_repository();
    let unresolved = direct_request(&repository, "symbol", "yo::missing", 8_000);
    let output = raw_resolve(&repository, &unresolved);
    assert!(output.stdout.is_empty());
    let failure: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(failure["error"]["code"], "explicit_anchor_unresolved");

    let over_budget = direct_request(&repository, "knowledge_id", "tui.context.large", 1);
    let failure = resolve_failure(&repository, &over_budget);
    assert_eq!(failure["error"]["code"], "required_budget_exceeded");
}

// 필수 지식이 stale하면 완전한 context가 아니므로 전체 요청을 실패시킨다.
// 선택 후보만 stale한 경우에는 요청을 살리되 해당 묶음을 제외한 이유를 bundle_stale로 남긴다.
#[test]
fn stale_required_knowledge_fails_and_stale_optional_candidate_is_omitted() {
    let repository = GitRepository::code_approved();
    repository.integrate_active_checkpoint();
    fs::write(
        repository.path.join("methexis/code-source.txt"),
        "changed after activation\n",
    )
    .unwrap();

    let required = direct_request(&repository, "knowledge_id", "tui.relocated", 8_000);
    let failure = resolve_failure(&repository, &required);
    assert_eq!(failure["error"]["code"], "required_knowledge_blocked");

    let optional = candidate_request(&repository, &[("tui.relocated", 100)], 8_000, false);
    let result = resolve(&repository, &optional);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            repository
                .path
                .join(result["manifest"]["path"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(result["affected_ids"], json!([]));
    assert_eq!(
        manifest["plan"]["candidate_decisions"][0]["reason"],
        "bundle_stale"
    );
}

// suspect는 stale보다 강한 검토 보류 상태다. 필수 anchor면 요청을 막고, 선택 후보면
// 요청 전체는 살리되 bundle_suspect라는 구체적 제외 사유를 남긴다.
#[test]
fn suspect_required_knowledge_fails_and_suspect_optional_candidate_is_omitted() {
    let repository = active_repository();
    let id = "tui.context.small";
    let revision = repository.revision_for(id);
    fs::write(
        repository.path.join("methexis/negative-records.yaml"),
        format!(
            "schema: methexis.negative-records/v1alpha1\nrecords:\n  - knowledge_id: {id}\n    revision: {revision}\n    condition: suspect\n    recorded_by: tui-architecture\n    evidence:\n      code: review.hold\n      reference: test://context/suspect\n"
        ),
    )
    .unwrap();

    let required = direct_request(&repository, "knowledge_id", id, 8_000);
    let failure = resolve_failure(&repository, &required);
    assert_eq!(failure["error"]["code"], "required_knowledge_blocked");

    let optional = candidate_request(&repository, &[(id, 100)], 8_000, false);
    let result = resolve(&repository, &optional);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            repository
                .path
                .join(result["manifest"]["path"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["plan"]["candidate_decisions"][0]["reason"],
        "bundle_suspect"
    );
}

// 같은 build id의 기존 디렉터리가 손상돼 있으면 정상 결과인 것처럼 덮어써서는 안 된다.
// 충돌을 실패로 보고하고 이번에 만든 임시 출력은 quarantine으로 옮겨 부분 게시를 막는다.
#[test]
fn corrupted_existing_build_fails_and_quarantines_new_output() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    let created = resolve(&repository, &request);
    let manifest = repository
        .path
        .join(created["manifest"]["path"].as_str().unwrap());
    fs::write(&manifest, b"corrupted\n").unwrap();

    let failure = resolve_failure(&repository, &request);

    assert_eq!(failure["error"]["code"], "context_build_collision");
    assert_eq!(fs::read(&manifest).unwrap(), b"corrupted\n");
    assert!(
        fs::read_dir(repository.path.join(".local-exclude/methexis/quarantine"))
            .unwrap()
            .next()
            .is_some()
    );
}

// 기존 build에 계약에 없는 파일이 있으면 다른 프로세스나 사람이 같은 위치를 바꾼 것이다.
// 그 내용을 지우거나 재사용하지 않고 context_build_collision으로 실패한다.
#[test]
fn existing_build_with_an_unexpected_file_is_a_collision() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    let created = resolve(&repository, &request);
    let build = repository
        .path
        .join(created["context"]["path"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_owned();
    fs::write(build.join("unexpected"), b"not part of the artifact set\n").unwrap();

    let failure = resolve_failure(&repository, &request);

    assert_eq!(failure["error"]["code"], "context_build_collision");
}

// candidate 입력이나 build 출력 경로에 symlink가 있으면 허용된 저장소 경계 밖을 읽거나 쓸 수 있다.
// 링크를 따라가지 않고 어느 경로가 안전하지 않은지 구조화된 오류로 보고한다.
#[test]
fn symlinked_candidate_and_build_paths_fail_closed() {
    let repository = active_repository();
    let outside = repository.path.join("outside.json");
    fs::write(&outside, b"{}\n").unwrap();
    let candidate_link = repository.path.join(".local-exclude/candidate-link.json");
    symlink(&outside, &candidate_link).unwrap();
    let request = repository.request(
        "symlink-candidate.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "candidates": {
                "path": ".local-exclude/candidate-link.json",
                "hash": format!("sha256:{}", "0".repeat(64))
            },
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": 8000
        }),
    );
    let failure = resolve_failure(&repository, &request);
    assert_eq!(failure["error"]["code"], "candidate_path_invalid");

    let direct = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    let created = resolve(&repository, &direct);
    let build = repository
        .path
        .join(created["context"]["path"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_owned();
    fs::remove_dir_all(&build).unwrap();
    let outside_directory = repository.path.join("outside-build");
    fs::create_dir(&outside_directory).unwrap();
    symlink(&outside_directory, &build).unwrap();

    let failure = resolve_failure(&repository, &direct);
    assert_eq!(failure["error"]["code"], "context_path_symlink");
}

// 지원하지 않는 tokenizer profile과 빈 요청은 구조화된 계약 오류로 실패하는지 확인한다.
#[test]
fn request_and_candidate_contract_failures_are_structured() {
    let repository = active_repository();
    let unsupported = repository.request(
        "unsupported-tokenizer.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "anchors": [{"kind": "knowledge_id", "value": "tui.context.base"}],
            "tokenizer_profile": "character-estimate/v1",
            "max_tokens": 8000
        }),
    );
    assert_eq!(
        resolve_failure(&repository, &unsupported)["error"]["code"],
        "unsupported_tokenizer_profile"
    );

    let empty = repository.request(
        "empty.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": 8000
        }),
    );
    assert_eq!(
        resolve_failure(&repository, &empty)["error"]["code"],
        "empty_context_request"
    );
}
