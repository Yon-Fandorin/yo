//! Agent-facing contract fixtures and the complete authoring path.

use std::{fs, path::Path};

use serde_json::{Value, json};

use super::support::*;

// author-revision 예제 요청으로 실제 CLI 명령을 실행하고 성공·실패 JSON 전체를 golden과 비교한다.
// 요청·응답 schema가 바뀌었는데 agent용 예제가 뒤처지는 문제를 이 비교로 잡는다.
#[test]
fn agent_contract_fixtures_are_complete_and_current() {
    let repository = TempRepository::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/author-contract");

    let actual = success_json(run(
        &repository.path,
        &[
            "author-revision",
            examples.join("request.json").to_str().unwrap(),
        ],
    ));
    let expected: Value =
        serde_json::from_slice(&fs::read(examples.join("success.json")).unwrap()).unwrap();
    assert_eq!(actual, expected);

    // 성공 예제가 revision을 바꾸므로 실패 예제는 원래 fixture 상태의 저장소에서 실행한다.
    let pristine = TempRepository::new();
    let actual = failure_json(run(
        &pristine.path,
        &[
            "author-revision",
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

// 세 가지 내용을 모두 바꾸는 요청 한 번으로 Source·Knowledge·Projection·packet 네 산출물이
// 모두 쓰이고, 기존 승인 제안은 그대로이며, 이후 check가 통과하는지 확인한다.
#[test]
fn authoring_a_full_revision_writes_every_derived_draft() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let approval_path = repository
        .path
        .join("methexis/approvals/tui.grapheme-cells.yaml");
    let approval_before = fs::read(&approval_path).expect("read approval");
    let request = repository.request("author.json", &author_request(&revision));

    let result = success_json(run(
        &repository.path,
        &["author-revision", request.to_str().unwrap()],
    ));

    assert_eq!(result["schema"], "methexis.author-revision/v1alpha1");
    assert_eq!(result["ok"], true);
    assert_eq!(result["operation"], "author-revision");
    assert_eq!(result["status"], "written");
    assert_eq!(result["authority"], "draft_proposal");
    assert_eq!(result["affected_ids"], json!([KNOWLEDGE_ID]));
    let new_revision = result["revision"].as_str().unwrap();
    assert!(new_revision.starts_with("sha256:"));
    assert_ne!(new_revision, revision);
    assert_eq!(
        result["changed_paths"],
        json!([
            "methexis/sources/decision/tui.fixture.yaml",
            "methexis/knowledge/tui.grapheme-cells.md",
            "methexis/review-projections/tui.grapheme-cells.md",
        ])
    );

    let source = fs::read_to_string(
        repository
            .path
            .join("methexis/sources/decision/tui.fixture.yaml"),
    )
    .expect("read source");
    assert!(source.contains("content: Cells are allocated per measured grapheme cluster."));
    let knowledge = fs::read_to_string(
        repository
            .path
            .join("methexis/knowledge/tui.grapheme-cells.md"),
    )
    .expect("read knowledge");
    assert!(knowledge.contains("one measured grapheme cluster"));
    assert!(knowledge.ends_with("cursor accounting.\n"));
    let projection = fs::read_to_string(
        repository
            .path
            .join("methexis/review-projections/tui.grapheme-cells.md"),
    )
    .expect("read projection");
    assert!(projection.contains(&format!("revision: {new_revision}")));
    let packet_manifest = repository
        .path
        .join(result["packet"]["path"].as_str().unwrap());
    assert!(packet_manifest.is_file());
    assert!(packet_manifest.with_file_name("packet.md").is_file());

    // 승인 기록은 authoring이 절대 건드리지 않는다.
    assert_eq!(
        fs::read(&approval_path).expect("read approval"),
        approval_before
    );

    // 파생값을 다시 계산하는 records·relations 검증이 작성 후에도 통과해야 한다.
    assert_eq!(repository.check()["ok"], true);
}

// 같은 요청을 다시 실행하면 이미 적용된 내용을 감지해 아무 파일도 다시 쓰지 않고
// 동일한 해시와 함께 status: unchanged를 반환하는지 확인한다.
#[test]
fn repeated_authoring_converges_to_unchanged() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let request = repository.request("author.json", &author_request(&revision));

    let first = success_json(run(
        &repository.path,
        &["author-revision", request.to_str().unwrap()],
    ));
    assert_eq!(first["status"], "written");
    let repeated = success_json(run(
        &repository.path,
        &["author-revision", request.to_str().unwrap()],
    ));

    assert_eq!(repeated["status"], "unchanged");
    assert_eq!(repeated["changed_paths"], json!([]));
    assert_eq!(repeated["revision"], first["revision"]);
    assert_eq!(repeated["projection_hash"], first["projection_hash"]);
    assert_eq!(repeated["packet"], first["packet"]);
    assert_eq!(repeated["request_hash"], first["request_hash"]);
}

// author-revision이 만든 packet은 같은 최종 상태에 대해 build-review가 만드는 packet과
// 바이트가 같아야 한다. content-addressed directory가 그대로 재사용되는지로 증명한다.
#[test]
fn authored_packet_matches_build_review_output() {
    let repository = TempRepository::new();
    let revision = repository.revision();
    let request = repository.request("author.json", &author_request(&revision));
    let authored = success_json(run(
        &repository.path,
        &["author-revision", request.to_str().unwrap()],
    ));

    let review_request = repository.request(
        "review.json",
        &json!({
            "schema": "methexis.review-request/v1alpha1",
            "knowledge_id": KNOWLEDGE_ID,
            "expected_revision": authored["revision"].as_str().unwrap(),
            "projection_hash": authored["projection_hash"].as_str().unwrap(),
        }),
    );
    let review = success_json(run(
        &repository.path,
        &["build-review", review_request.to_str().unwrap()],
    ));

    assert_eq!(review["status"], "unchanged");
    assert_eq!(review["path"], authored["packet"]["path"]);
    assert_eq!(review["hash"], authored["packet"]["hash"]);
    let manifest: Value = serde_json::from_slice(
        &fs::read(repository.path.join(review["path"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["request_hash"], review["request_hash"]);
}
