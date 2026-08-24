//! Agent-facing contract fixtures and the complete happy path.

use std::{fs, path::Path};

use serde_json::{Value, json};

use super::support::*;

// 각 review 예제 요청으로 실제 CLI 명령을 실행하고 성공·실패 JSON 전체를 golden과 비교한다.
// 요청·응답 schema가 바뀌었는데 agent용 예제가 뒤처지는 문제를 이 비교로 잡는다.
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

// 같은 원문 revision으로 projection을 만들고 검토·승인 제안을 다시 실행해도 중복 산출물이 생기면
// 안 된다. 기존 Draft 제안을 안전하게 재사용해 결과가 동일한 idempotent 흐름인지 확인한다.
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

// canonical basis는 exact 영문 revision만 승인 증거로 사용하며 Projection 파일이나
// review packet을 생성·요구하지 않는다.
#[test]
fn canonical_approval_needs_no_projection_and_remains_idempotent() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let request = repository.request(
        "canonical-approval.json",
        &canonical_approval_request(&revision, "tui-architecture", "2026-07-24T12:00:00Z"),
    );

    let approval = success_json(run(
        &repository.path,
        &["approve", request.to_str().unwrap()],
    ));
    assert_eq!(approval["status"], "written");
    let repeated = success_json(run(
        &repository.path,
        &["approve", request.to_str().unwrap()],
    ));
    assert_eq!(repeated["status"], "unchanged");
    assert!(
        !repository
            .path
            .join("methexis/review-projections/tui.relocated.md")
            .exists()
    );
    let record = fs::read_to_string(
        repository
            .path
            .join("methexis/approvals/tui.relocated.yaml"),
    )
    .unwrap();
    assert!(record.contains("schema: methexis.approval/v1alpha2"));
    assert!(record.contains("review_basis: canonical"));
    assert!(!record.contains("projection_"));

    let check = repository.check();
    assert_eq!(check["units"][0]["approval_evidence"], "matching_proposal");
}
