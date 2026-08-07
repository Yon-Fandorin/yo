//! Input, shape, and validation failures for revision authoring.

use std::fs;

use serde_json::{Value, json};

use super::support::*;

fn author_failure(repository: &TempRepository, request: &Value) -> Value {
    let request = repository.request("failure.json", request);
    failure_json(run(
        &repository.path,
        &["author-revision", request.to_str().unwrap()],
    ))
}

fn assert_failure(result: &Value, code: &str) {
    assert_eq!(result["schema"], "methexis.operation/v1alpha1");
    assert_eq!(result["ok"], false);
    assert_eq!(result["operation"], "author-revision");
    assert_eq!(result["error"]["code"], code);
}

// expected_revision은 현재 파생 revision에 대한 compare-and-swap 전제 조건이다.
#[test]
fn stale_expected_revision_fails_with_revision_mismatch() {
    let repository = TempRepository::new();
    let request =
        author_request("sha256:0000000000000000000000000000000000000000000000000000000000000000");
    let result = author_failure(&repository, &request);

    assert_failure(&result, "revision_mismatch");
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains(&repository.revision())
    );
}

// 대상 KnowledgeId가 없으면 어떤 파일도 쓰지 않고 실패한다.
#[test]
fn unknown_knowledge_id_is_rejected() {
    let repository = TempRepository::new();
    let mut request = author_request(&repository.revision());
    request["knowledge_id"] = json!("tui.missing");

    assert_failure(
        &author_failure(&repository, &request),
        "unknown_knowledge_id",
    );
}

// 첫 버전은 decision Source 하나만 지원한다. 여러 Source를 pin한 unit은 닫힌 실패를 반환한다.
#[test]
fn multi_source_units_fail_closed() {
    let repository = TempRepository::new();
    let request = json!({
        "schema": "methexis.author-revision-request/v1alpha1",
        "knowledge_id": MULTI_SOURCE_ID,
        "expected_revision": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "korean_markdown": "무엇이든.",
    });
    let result = author_failure(&repository, &request);

    assert_failure(&result, "unsupported_unit_shape");
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("single decision Source")
    );
}

// 세 내용 필드가 모두 빠진 요청은 아무 파생도 하지 않고 거부한다.
#[test]
fn a_request_without_any_content_field_is_empty() {
    let repository = TempRepository::new();
    let request = json!({
        "schema": "methexis.author-revision-request/v1alpha1",
        "knowledge_id": KNOWLEDGE_ID,
        "expected_revision": repository.revision(),
    });

    assert_failure(
        &author_failure(&repository, &request),
        "empty_author_request",
    );
}

// 각 내용 필드는 trim 후 비어 있으면 안 된다.
#[test]
fn blank_content_fields_fail_individually() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    for (field, code) in [
        ("source_content", "empty_source_content"),
        ("knowledge_body", "empty_knowledge_body"),
        ("korean_markdown", "empty_korean_markdown"),
    ] {
        let mut request = json!({
            "schema": "methexis.author-revision-request/v1alpha1",
            "knowledge_id": KNOWLEDGE_ID,
            "expected_revision": revision,
        });
        request[field] = json!("  \r\n  ");
        assert_failure(&author_failure(&repository, &request), code);
    }
}

// 알 수 없는 필드나 잘못된 schema 버전은 request 계층에서 거부한다.
#[test]
fn malformed_requests_fail_before_any_write() {
    let repository = TempRepository::new();
    let revision = repository.revision();

    let mut unknown_field = author_request(&revision);
    unknown_field["surprise"] = json!(true);
    assert_failure(
        &author_failure(&repository, &unknown_field),
        "invalid_request",
    );

    let mut wrong_schema = author_request(&revision);
    wrong_schema["schema"] = json!("methexis.author-revision-request/v0");
    assert_failure(
        &author_failure(&repository, &wrong_schema),
        "unsupported_request_schema",
    );

    let missing = failure_json(run(
        &repository.path,
        &["author-revision", "no-such-request.json"],
    ));
    assert_failure(&missing, "request_unreadable");
}

// 새 Knowledge 본문은 쓰기 전에 기존 validator를 통과해야 한다.
#[test]
fn invalid_knowledge_bodies_fail_before_publication() {
    let repository = TempRepository::new();
    let revision = repository.revision();

    let missing_section = json!({
        "schema": "methexis.author-revision-request/v1alpha1",
        "knowledge_id": KNOWLEDGE_ID,
        "expected_revision": revision,
        "knowledge_body": "No sections at all.",
    });
    assert_failure(
        &author_failure(&repository, &missing_section),
        "missing_body_section",
    );

    let raw_html = json!({
        "schema": "methexis.author-revision-request/v1alpha1",
        "knowledge_id": KNOWLEDGE_ID,
        "expected_revision": revision,
        "knowledge_body": "## Statement\n\n<div>raw</div>\n\n## Rationale\n\nBecause.\n",
    });
    assert_failure(
        &author_failure(&repository, &raw_html),
        "raw_html_forbidden",
    );

    // 실패 후에도 fixture는 그대로여야 한다.
    assert_eq!(repository.check()["ok"], true);
}

// 쓰기 시퀀스 중간에(Source·Knowledge 기록 후 Projection 기록 전) 실패하면 실패 JSON이
// 이미 쓴 경로를 보고하고, 장애물을 제거한 뒤 같은 요청을 다시 실행하면 나머지만 쓰면서
// clean run과 동일한 파생 revision으로 수렴한다.
#[test]
fn mid_sequence_failure_reports_written_paths_and_rerun_converges() {
    use std::os::unix::fs::PermissionsExt;

    let repository = TempRepository::new();
    let clean = TempRepository::new();
    let revision = repository.revision();
    let request = author_request(&revision);
    let request_path = repository.request("partial.json", &request);
    let clean_request = clean.request("clean.json", &request);

    let expected = success_json(run(
        &clean.path,
        &["author-revision", clean_request.to_str().unwrap()],
    ));

    // Projection 출력 디렉터리를 읽기 전용으로 만들어 Source·Knowledge 쓰기는 성공하고
    // 그 뒤의 Projection 쓰기는 실패하게 한다.
    let projections = repository.path.join("methexis/review-projections");
    let writable = fs::metadata(&projections).unwrap().permissions();
    fs::set_permissions(&projections, fs::Permissions::from_mode(0o555)).unwrap();

    let failure = failure_json(run(
        &repository.path,
        &["author-revision", request_path.to_str().unwrap()],
    ));

    // 임시 저장소 정리(Drop)가 가능하도록 권한을 먼저 되돌린다.
    fs::set_permissions(&projections, writable).unwrap();

    assert_failure(&failure, "publication_failed");
    let message = failure["error"]["message"].as_str().unwrap();
    assert!(
        message.contains(
            "already wrote: methexis/sources/decision/tui.fixture.yaml, methexis/knowledge/tui.grapheme-cells.md"
        ),
        "message: {message}"
    );
    assert_eq!(
        failure["error"]["next_actions"],
        json!(["re-run the same request to converge the remaining writes"])
    );

    // 부분 상태: Source·Knowledge는 새 내용인데 Projection은 이전 revision 그대로다.
    let source = fs::read_to_string(
        repository
            .path
            .join("methexis/sources/decision/tui.fixture.yaml"),
    )
    .unwrap();
    assert!(source.contains("content: Cells are allocated per measured grapheme cluster."));
    let projection = fs::read_to_string(projections.join("tui.grapheme-cells.md")).unwrap();
    assert!(projection.contains(&format!("revision: {revision}")));

    let rerun = success_json(run(
        &repository.path,
        &["author-revision", request_path.to_str().unwrap()],
    ));
    assert_eq!(rerun["status"], "written");
    assert_eq!(
        rerun["changed_paths"],
        json!(["methexis/review-projections/tui.grapheme-cells.md"])
    );
    assert_eq!(rerun["revision"], expected["revision"]);
    assert_eq!(rerun["projection_hash"], expected["projection_hash"]);
    assert_eq!(rerun["packet"], expected["packet"]);

    // 수렴 후 파생값을 다시 계산하는 검증도 통과해야 한다.
    assert_eq!(repository.check()["ok"], true);
}
