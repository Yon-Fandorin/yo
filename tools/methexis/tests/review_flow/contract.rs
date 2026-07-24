//! Agent-facing contract fixtures and the complete happy path.

use std::{fs, path::Path};

use serde_json::{Value, json};

use super::support::*;

#[test]
fn agent_contract_fixtures_are_complete_and_current() {
    let repository = TempRepository::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/review-contract");

    for (command, request, expected) in [
        (
            "project-review",
            "projection-request.json",
            "projection-success.json",
        ),
        ("build-review", "review-request.json", "review-success.json"),
        ("approve", "approval-request.json", "approval-success.json"),
    ] {
        let actual = success_json(run(
            &repository.path,
            &[command, examples.join(request).to_str().unwrap()],
        ));
        let expected: Value =
            serde_json::from_slice(&fs::read(examples.join(expected)).unwrap()).unwrap();
        assert_eq!(actual, expected);
    }

    let actual = failure_json(run(
        &repository.path,
        &[
            "project-review",
            examples
                .join("revision-mismatch-request.json")
                .to_str()
                .unwrap(),
        ],
    ));
    let expected: Value =
        serde_json::from_slice(&fs::read(examples.join("revision-mismatch.json")).unwrap())
            .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn projection_review_and_approval_form_an_idempotent_proposal_flow() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let projection_request = repository.request(
        "projection.json",
        &projection_request(&revision, "물리적 위치는 의미적 정체성이 아닙니다."),
    );

    let projection = success_json(run(
        &repository.path,
        &["project-review", projection_request.to_str().unwrap()],
    ));
    assert_eq!(projection["status"], "written");
    assert_eq!(projection["authority"], "draft_proposal");
    let projection_hash = projection["hash"].as_str().unwrap().to_owned();
    let repeated = success_json(run(
        &repository.path,
        &["project-review", projection_request.to_str().unwrap()],
    ));
    assert_eq!(repeated["status"], "unchanged");
    assert_eq!(repeated["hash"], projection_hash);

    let review_request = repository.request(
        "review.json",
        &json!({
            "schema": "methexis.review-request/v1alpha1",
            "knowledge_id": KNOWLEDGE_ID,
            "expected_revision": revision,
            "projection_hash": projection_hash,
        }),
    );
    let review = success_json(run(
        &repository.path,
        &["build-review", review_request.to_str().unwrap()],
    ));
    assert_eq!(review["status"], "written");
    let repeated_review = success_json(run(
        &repository.path,
        &["build-review", review_request.to_str().unwrap()],
    ));
    assert_eq!(repeated_review["status"], "unchanged");
    assert_eq!(repeated_review["path"], review["path"]);
    let manifest = repository.path.join(review["path"].as_str().unwrap());
    let packet = fs::read_to_string(manifest.parent().unwrap().join("packet.md"))
        .expect("read review packet");
    assert!(packet.contains("## Canonical English"));
    assert!(packet.contains("## Korean Review Projection"));
    assert!(packet.contains("Source validation: `not_evaluated`"));

    let approval_request = repository.request(
        "approval.json",
        &json!({
            "schema": "methexis.approval-request/v1alpha1",
            "knowledge_id": KNOWLEDGE_ID,
            "expected_revision": revision,
            "projection_hash": projection_hash,
            "reviewer": "tui-architecture",
            "reviewed_at": "2026-07-24T12:00:00Z",
        }),
    );
    let approval = success_json(run(
        &repository.path,
        &["approve", approval_request.to_str().unwrap()],
    ));
    assert_eq!(approval["status"], "written");
    assert_eq!(approval["authority"], "draft_proposal");
    let repeated = success_json(run(
        &repository.path,
        &["approve", approval_request.to_str().unwrap()],
    ));
    assert_eq!(repeated["status"], "unchanged");

    let check = repository.check();
    assert_eq!(check["approval"], "proposal_evaluated");
    assert_eq!(check["units"][0]["effective_approval"], "draft");
    assert_eq!(check["units"][0]["approval_evidence"], "matching_proposal");
    assert_eq!(check["units"][0]["approval_reason"], Value::Null);
}
