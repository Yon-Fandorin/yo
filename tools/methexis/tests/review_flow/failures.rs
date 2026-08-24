//! Structural, evidence, input, and publication failure scenarios.

use std::fs;

use serde_json::json;

use super::support::*;

// 요청 revision이 원문과 다르면 projection을 만들지 않고, projection 증거가 없으면 approval도
// 만들지 않는다. 생성 뒤 원문이 바뀐 projection은 stale로 보고하되 새 권한을 부여하지 않는다.
#[test]
fn invalid_requests_and_stale_projection_fail_without_partial_authority() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let bad_revision = repository.request(
        "bad-revision.json",
        &projection_request(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "번역",
        ),
    );
    let failure = failure_json(run(
        &repository.path,
        &["project-review", bad_revision.to_str().unwrap()],
    ));
    assert_eq!(failure["error"]["code"], "revision_mismatch");
    assert!(
        !repository
            .path
            .join("methexis/review-projections/tui.relocated.md")
            .exists()
    );

    let missing_projection_approval = repository.request(
        "missing-projection.json",
        &json!({
            "schema": "methexis.approval-request/v1alpha1",
            "knowledge_id": KNOWLEDGE_ID,
            "expected_revision": revision,
            "projection_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "reviewer": "tui-architecture",
            "reviewed_at": "2026-07-24T12:00:00Z",
        }),
    );
    let failure = failure_json(run(
        &repository.path,
        &["approve", missing_projection_approval.to_str().unwrap()],
    ));
    assert_eq!(failure["error"]["code"], "projection_unreadable");
    assert!(
        !repository
            .path
            .join("methexis/approvals/tui.relocated.yaml")
            .exists()
    );

    let projection_request = repository.request(
        "projection.json",
        &projection_request(&revision, "검토할 번역입니다."),
    );
    success_json(run(
        &repository.path,
        &["project-review", projection_request.to_str().unwrap()],
    ));
    let knowledge = repository
        .path
        .join("methexis/knowledge/first-location/unit.md");
    let mut changed = fs::read_to_string(&knowledge).unwrap();
    changed.push_str("\nMeaning changed.\n");
    fs::write(knowledge, changed).unwrap();

    let check = success_json(run(&repository.path, &["check"]));
    assert_eq!(check["units"][0]["approval_evidence"], "missing");
    assert_eq!(check["units"][0]["approval_reason"], "missing_approval");
}

// fresh canonical approval은 선택적 stale Projection을 증거로 쓰지 않지만, Projection 자체가
// 직접 편집되어 구조가 깨지면 저장소 무결성 검사는 계속 fail closed 한다.
#[test]
fn stale_projection_is_optional_for_canonical_approval_but_malformed_projection_still_fails() {
    let repository = TempRepository::new();
    let old_revision = repository.revision();
    let projection_request = repository.request(
        "old-projection.json",
        &projection_request(&old_revision, "이전 revision의 선택적 번역입니다."),
    );
    success_json(run(
        &repository.path,
        &["project-review", projection_request.to_str().unwrap()],
    ));
    let knowledge = repository
        .path
        .join("methexis/knowledge/first-location/unit.md");
    let mut changed = fs::read_to_string(&knowledge).unwrap();
    changed.push_str("\nCanonical meaning is now more explicit.\n");
    fs::write(&knowledge, changed).unwrap();
    let revision = repository.revision();
    let request = repository.request(
        "canonical.json",
        &canonical_approval_request(&revision, "tui-architecture", "2026-07-24T12:00:00Z"),
    );
    success_json(run(
        &repository.path,
        &["approve", request.to_str().unwrap()],
    ));

    let check = repository.check();
    assert_eq!(check["units"][0]["approval_evidence"], "matching_proposal");

    let projection_path = repository
        .path
        .join("methexis/review-projections/tui.relocated.md");
    let mut damaged = fs::read_to_string(&projection_path).unwrap();
    damaged.push_str("\n직접 편집된 내용\n");
    fs::write(projection_path, damaged).unwrap();
    let failure = failure_json(run(&repository.path, &["check"]));
    assert!(has_diagnostic(&failure, "projection_lineage_mismatch"));
}

// 안전하지 않거나 쓸 수 없는 출력 부모에는 임시 projection조차 남기지 않는다.
#[test]
fn unsafe_or_unwritable_output_parent_leaves_no_partial_projection() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    fs::write(
        repository.path.join("methexis/review-projections"),
        b"not a directory",
    )
    .expect("create blocking file");
    let request = repository.request("projection.json", &projection_request(&revision, "번역"));

    let failure = failure_json(run(
        &repository.path,
        &["project-review", request.to_str().unwrap()],
    ));

    assert_eq!(failure["error"]["code"], "publication_failed");
    assert_eq!(
        fs::read(repository.path.join("methexis/review-projections")).unwrap(),
        b"not a directory"
    );
}

// reviewer·evidence·시간이 계약과 다르면 approval 생성을 모두 거부한다.
#[test]
fn approval_rejects_wrong_evidence_reviewer_and_time() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let projection_request = repository.request(
        "projection.json",
        &projection_request(&revision, "검토 번역입니다."),
    );
    let projection = success_json(run(
        &repository.path,
        &["project-review", projection_request.to_str().unwrap()],
    ));
    let projection_hash = projection["hash"].as_str().unwrap();

    for (name, request, expected_code) in [
        (
            "wrong-hash.json",
            approval_request(
                &revision,
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "tui-architecture",
                "2026-07-24T12:00:00Z",
            ),
            "projection_mismatch",
        ),
        (
            "unknown-reviewer.json",
            approval_request(
                &revision,
                projection_hash,
                "unknown-owner",
                "2026-07-24T12:00:00Z",
            ),
            "unknown_reviewer",
        ),
        (
            "invalid-time.json",
            approval_request(
                &revision,
                projection_hash,
                "tui-architecture",
                "2026-02-30T12:00:00Z",
            ),
            "invalid_review_time",
        ),
    ] {
        let request = repository.request(name, &request);
        let failure = failure_json(run(
            &repository.path,
            &["approve", request.to_str().unwrap()],
        ));
        assert_eq!(failure["error"]["code"], expected_code);
    }
    assert!(
        !repository
            .path
            .join("methexis/approvals/tui.relocated.yaml")
            .exists()
    );
}

// v1alpha2는 canonical basis만을 명시하는 요청이다. 다른 basis를 넣어 Projection 경로로
// 암묵 전환하거나 fallback하지 않는다.
#[test]
fn canonical_request_rejects_a_different_basis_without_fallback() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let request = repository.request(
        "wrong-basis.json",
        &json!({
            "schema": "methexis.approval-request/v1alpha2",
            "knowledge_id": KNOWLEDGE_ID,
            "expected_revision": revision,
            "review_basis": "projection",
            "reviewer": "tui-architecture",
            "reviewed_at": "2026-07-24T12:00:00Z",
        }),
    );

    let failure = failure_json(run(
        &repository.path,
        &["approve", request.to_str().unwrap()],
    ));
    assert_eq!(failure["error"]["code"], "invalid_request");
    assert!(
        !repository
            .path
            .join("methexis/approvals/tui.relocated.yaml")
            .exists()
    );
}

// 생성 뒤 사람이 고친 projection과 내부 hash가 깨진 approval은 둘 다 신뢰할 수 없다.
// 단순한 stale 상태로 취급하지 않고 구조적 무결성 실패로 명확히 거부한다.
#[test]
fn edited_projection_and_damaged_approval_are_structural_failures() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let initial_projection_request = repository.request(
        "projection.json",
        &projection_request(&revision, "원래 검토한 번역입니다."),
    );
    success_json(run(
        &repository.path,
        &[
            "project-review",
            initial_projection_request.to_str().unwrap(),
        ],
    ));
    let projection_path = repository
        .path
        .join("methexis/review-projections/tui.relocated.md");
    let mut edited = fs::read_to_string(&projection_path).unwrap();
    edited.push_str("\n검토되지 않은 수정입니다.\n");
    fs::write(&projection_path, edited).unwrap();
    let failure = failure_json(run(&repository.path, &["check"]));
    assert!(has_diagnostic(&failure, "projection_lineage_mismatch"));

    fs::remove_file(&projection_path).unwrap();
    let replacement_projection_request = repository.request(
        "projection-recreated.json",
        &projection_request(&revision, "원래 검토한 번역입니다."),
    );
    let projection = success_json(run(
        &repository.path,
        &[
            "project-review",
            replacement_projection_request.to_str().unwrap(),
        ],
    ));
    let approval_request = repository.request(
        "approval.json",
        &approval_request(
            &revision,
            projection["hash"].as_str().unwrap(),
            "tui-architecture",
            "2026-07-24T12:00:00Z",
        ),
    );
    success_json(run(
        &repository.path,
        &["approve", approval_request.to_str().unwrap()],
    ));
    let approval_path = repository
        .path
        .join("methexis/approvals/tui.relocated.yaml");
    let original = fs::read_to_string(&approval_path).unwrap();
    let changed_time = original.replace(
        "reviewed_at: 2026-07-24T12:00:00Z",
        "reviewed_at: 2026-07-24T13:00:00Z",
    );
    fs::write(&approval_path, changed_time).unwrap();
    let failure = failure_json(run(&repository.path, &["check"]));
    assert!(has_diagnostic(&failure, "approval_lineage_mismatch"));

    fs::write(&approval_path, original).unwrap();
    let mut damaged = fs::read_to_string(&approval_path).unwrap();
    damaged.push_str("unexpected: field\n");
    fs::write(approval_path, damaged).unwrap();
    let failure = failure_json(run(&repository.path, &["check"]));
    assert!(has_diagnostic(&failure, "invalid_approval_yaml"));
}

// 보이는 raw HTML은 projection에서 거부하되 fenced code 안의 HTML 예시는 허용한다.
#[test]
fn projection_rejects_visible_raw_html_but_allows_fenced_examples() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let raw_request = repository.request(
        "raw-html.json",
        &projection_request(&revision, "<details>숨겨진 의미</details>"),
    );
    let failure = failure_json(run(
        &repository.path,
        &["project-review", raw_request.to_str().unwrap()],
    ));
    assert_eq!(failure["error"]["code"], "raw_html_forbidden");

    let fenced_request = repository.request(
        "fenced-html.json",
        &projection_request(&revision, "```html\n<details>예시</details>\n```"),
    );
    let projection = success_json(run(
        &repository.path,
        &["project-review", fenced_request.to_str().unwrap()],
    ));
    assert_eq!(projection["status"], "written");
}

#[cfg(unix)]
// projection 대상이 symlink면 그 내용을 읽거나 교체하지 않고 안전하게 거부한다.
#[test]
fn projection_refuses_a_symlink_target_without_reading_or_replacing_it() {
    use std::os::unix::fs::symlink;

    let repository = TempRepository::new();
    let revision = repository.revision();
    let output = repository.path.join("methexis/review-projections");
    fs::create_dir(&output).unwrap();
    let request = repository.request(
        "projection.json",
        &projection_request(&revision, "검토 번역"),
    );
    let outside = repository.path.join(".local-exclude/outside.md");
    fs::write(&outside, b"outside bytes").unwrap();
    let target = output.join("tui.relocated.md");
    symlink(&outside, &target).unwrap();

    let failure = failure_json(run(
        &repository.path,
        &["project-review", request.to_str().unwrap()],
    ));

    assert_eq!(failure["error"]["code"], "symlink_forbidden");
    assert_eq!(fs::read(&outside).unwrap(), b"outside bytes");
    assert!(
        fs::symlink_metadata(target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
