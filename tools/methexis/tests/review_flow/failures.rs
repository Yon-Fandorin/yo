//! Structural, evidence, input, and publication failure scenarios.

use std::fs;

use serde_json::json;

use super::support::*;

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

    let failure = failure_json(run(&repository.path, &["check"]));
    assert!(
        failure["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "stale_review_projection")
    );
}

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
