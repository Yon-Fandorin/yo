//! Projection hash and approval revision compare-and-swap scenarios.

use std::fs;

use serde_json::json;

use super::support::*;

// projection 교체는 기존 projection의 정확한 hash를 제시한 경우에만 허용한다.
#[test]
fn projection_replacement_requires_the_exact_existing_hash() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let first_request = repository.request(
        "first.json",
        &projection_request(&revision, "첫 번째 번역입니다."),
    );
    let first = success_json(run(
        &repository.path,
        &["project-review", first_request.to_str().unwrap()],
    ));
    let first_hash = first["hash"].as_str().unwrap();
    let target = repository
        .path
        .join("methexis/review-projections/tui.relocated.md");
    let first_bytes = fs::read(&target).expect("read first Projection");

    let conflict_request = repository.request(
        "conflict.json",
        &projection_request(&revision, "두 번째 번역입니다."),
    );
    let conflict = failure_json(run(
        &repository.path,
        &["project-review", conflict_request.to_str().unwrap()],
    ));
    assert_eq!(conflict["error"]["code"], "replacement_conflict");
    assert_eq!(fs::read(&target).unwrap(), first_bytes);

    let replacement_request = repository.request(
        "replacement.json",
        &json!({
            "schema": "methexis.review-projection-request/v1alpha1",
            "knowledge_id": KNOWLEDGE_ID,
            "expected_revision": revision,
            "korean_markdown": "두 번째 번역입니다.",
            "replace_projection_hash": first_hash,
        }),
    );
    let replacement = success_json(run(
        &repository.path,
        &["project-review", replacement_request.to_str().unwrap()],
    ));
    assert_eq!(replacement["status"], "written");
    assert_ne!(replacement["hash"], first_hash);
}

// approval 교체는 바로 이전 승인 revision을 정확히 지정한 경우에만 허용한다.
#[test]
fn approval_replacement_requires_the_exact_previous_revision() {
    let repository = TempRepository::new();
    let first_revision = repository.revision();
    let first_projection_request = repository.request(
        "projection-v1.json",
        &projection_request(&first_revision, "첫 번째 리비전입니다."),
    );
    let first_projection = success_json(run(
        &repository.path,
        &["project-review", first_projection_request.to_str().unwrap()],
    ));
    let first_projection_hash = first_projection["hash"].as_str().unwrap();
    let first_approval_request = repository.request(
        "approval-v1.json",
        &approval_request(
            &first_revision,
            first_projection_hash,
            "tui-architecture",
            "2026-07-24T12:00:00Z",
        ),
    );
    success_json(run(
        &repository.path,
        &["approve", first_approval_request.to_str().unwrap()],
    ));

    let knowledge = repository
        .path
        .join("methexis/knowledge/first-location/unit.md");
    let mut changed = fs::read_to_string(&knowledge).unwrap();
    changed.push_str("\nThe clarified outcome is still independently relocatable.\n");
    fs::write(&knowledge, changed).unwrap();
    let first_projection_path = repository
        .path
        .join("methexis/review-projections/tui.relocated.md");
    let approval_path = repository
        .path
        .join("methexis/approvals/tui.relocated.yaml");
    let parked_projection = repository.path.join(".local-exclude/projection-v1.md");
    let parked_approval = repository.path.join(".local-exclude/approval-v1.yaml");
    fs::rename(&first_projection_path, &parked_projection).unwrap();
    fs::rename(&approval_path, &parked_approval).unwrap();
    let second_revision = repository.revision();
    assert_ne!(second_revision, first_revision);
    fs::rename(&parked_projection, &first_projection_path).unwrap();
    fs::rename(&parked_approval, &approval_path).unwrap();

    let old_projection_hash = sha256(&fs::read(&first_projection_path).unwrap());
    let second_projection_request = repository.request(
        "projection-v2.json",
        &json!({
            "schema": "methexis.review-projection-request/v1alpha1",
            "knowledge_id": KNOWLEDGE_ID,
            "expected_revision": second_revision,
            "korean_markdown": "두 번째 리비전입니다.",
            "replace_projection_hash": old_projection_hash,
        }),
    );
    let second_projection = success_json(run(
        &repository.path,
        &[
            "project-review",
            second_projection_request.to_str().unwrap(),
        ],
    ));
    let second_projection_hash = second_projection["hash"].as_str().unwrap();
    let first_approval_bytes = fs::read(&approval_path).unwrap();

    let conflict_request = repository.request(
        "approval-v2-conflict.json",
        &approval_request(
            &second_revision,
            second_projection_hash,
            "tui-architecture",
            "2026-07-24T13:00:00Z",
        ),
    );
    let failure = failure_json(run(
        &repository.path,
        &["approve", conflict_request.to_str().unwrap()],
    ));
    assert_eq!(failure["error"]["code"], "approval_replacement_conflict");
    assert_eq!(fs::read(&approval_path).unwrap(), first_approval_bytes);

    let replacement_request = repository.request(
        "approval-v2.json",
        &json!({
            "schema": "methexis.approval-request/v1alpha1",
            "knowledge_id": KNOWLEDGE_ID,
            "expected_revision": second_revision,
            "projection_hash": second_projection_hash,
            "reviewer": "tui-architecture",
            "reviewed_at": "2026-07-24T13:00:00Z",
            "replace_revision": first_revision,
        }),
    );
    let replacement = success_json(run(
        &repository.path,
        &["approve", replacement_request.to_str().unwrap()],
    ));
    assert_eq!(replacement["status"], "written");
    assert_eq!(
        repository.check()["units"][0]["approval_evidence"],
        "matching_proposal"
    );
}
