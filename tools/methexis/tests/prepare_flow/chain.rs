//! Chained prepare commands across the full review-to-activation loop.

use std::fs;

use serde_json::json;

use super::support::*;

// 활성 Checkpoint를 가진 승인 저장소에서 prepare-approval → approve → trusted 통합 →
// prepare-checkpoint → create-checkpoint → prepare-activation → propose-activation까지
// 수동 복사 없이 prepare 출력만으로 전체 루프를 잇는다. 마지막 propose-activation 성공이
// prepare-activation이 계산한 compare-and-swap predecessor 해시가 정확함을 증명한다.
#[test]
fn prepare_commands_chain_the_full_review_activation_loop() {
    let repository = GitRepository::approved();
    repository.integrate_active_checkpoint();

    let revision = repository.revision_for(KNOWLEDGE_ID);
    let projection = repository.project(KNOWLEDGE_ID, &revision);
    let manifest = repository.build_manifest(
        KNOWLEDGE_ID,
        &revision,
        projection["hash"].as_str().unwrap(),
    );

    let approval_path = repository
        .path
        .join(format!("methexis/approvals/{KNOWLEDGE_ID}.yaml"));
    let approval_before = fs::read(&approval_path).expect("read current approval");
    let prepared = success_json(repository.run(&[
        "prepare-approval",
        &manifest,
        "--reviewer",
        OWNER_ID,
        "--replace-current",
    ]));
    assert_eq!(prepared["schema"], "methexis.approval-request/v1alpha1");
    assert_eq!(prepared["knowledge_id"], KNOWLEDGE_ID);
    assert_eq!(prepared["expected_revision"], revision);
    assert_eq!(prepared["projection_hash"], projection["hash"]);
    assert_eq!(prepared["reviewer"], OWNER_ID);
    assert_eq!(prepared["replace_revision"], revision);

    // prepare는 approvals/를 쓰지 않는다. 기록은 사람이 명시적으로 approve를 실행할 때만 일어난다.
    assert_eq!(
        fs::read(&approval_path).expect("read current approval"),
        approval_before
    );

    let approval_request = repository.request("approval.json", &prepared);
    let approval = success_json(repository.run(&["approve", approval_request.to_str().unwrap()]));
    assert_eq!(approval["status"], "written");
    repository.git(&["add", "methexis"]);
    repository.git(&["commit", "-m", "integrate replacement approval"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let prepared_checkpoint = success_json(repository.run(&["prepare-checkpoint"]));
    assert_eq!(
        prepared_checkpoint["schema"],
        "methexis.checkpoint-request/v1alpha1"
    );
    assert_eq!(prepared_checkpoint["roots"], json!([KNOWLEDGE_ID]));
    let checkpoint_request = repository.request("checkpoint.json", &prepared_checkpoint);
    let created =
        success_json(repository.run(&["create-checkpoint", checkpoint_request.to_str().unwrap()]));
    assert_eq!(created["status"], "written");

    let create_output = repository.request("create-output.json", &created);
    let prepared_activation =
        success_json(repository.run(&["prepare-activation", create_output.to_str().unwrap()]));
    assert_eq!(
        prepared_activation["schema"],
        "methexis.activation-request/v1alpha1"
    );
    assert_eq!(
        prepared_activation["checkpoint_id"],
        created["checkpoint_id"]
    );
    assert_eq!(prepared_activation["checkpoint_hash"], created["hash"]);
    assert!(
        prepared_activation["replace_active_hash"]
            .as_str()
            .expect("replacement carries the active predecessor hash")
            .starts_with("sha256:")
    );

    let activation_request = repository.request("activation.json", &prepared_activation);
    let activation =
        success_json(repository.run(&["propose-activation", activation_request.to_str().unwrap()]));
    assert_eq!(activation["status"], "written");
}

// 최초 승인 루프에서는 prepare-approval이 replace_revision 없이 요청을 방출하고,
// 방출 자체는 approvals/에 아무 파일도 쓰지 않으며, 그 출력을 그대로 approve에 먹일 수 있다.
#[test]
fn initial_preparation_omits_replacement_and_writes_nothing() {
    let repository = GitRepository::foundation();
    let revision = repository.revision_for(KNOWLEDGE_ID);
    let projection = repository.project(KNOWLEDGE_ID, &revision);
    let manifest = repository.build_manifest(
        KNOWLEDGE_ID,
        &revision,
        projection["hash"].as_str().unwrap(),
    );
    let approval_path = repository
        .path
        .join(format!("methexis/approvals/{KNOWLEDGE_ID}.yaml"));

    let prepared =
        success_json(repository.run(&["prepare-approval", &manifest, "--reviewer", OWNER_ID]));

    assert_eq!(prepared["schema"], "methexis.approval-request/v1alpha1");
    assert_eq!(prepared["expected_revision"], revision);
    assert!(prepared.get("replace_revision").is_none());
    assert!(!approval_path.exists());

    let approval_request = repository.request("approval.json", &prepared);
    let approval = success_json(repository.run(&["approve", approval_request.to_str().unwrap()]));
    assert_eq!(approval["status"], "written");
    assert!(approval_path.is_file());
}

// canonical prepare 출력은 Projection 없이 approve와 initial Checkpoint/activation 제안까지
// 이어져 capability가 단순 CLI 광고가 아니라 완전한 최소 흐름임을 증명한다.
#[test]
fn canonical_preparation_chains_to_initial_activation_without_a_projection() {
    let repository = GitRepository::foundation();
    let revision = repository.revision_for(KNOWLEDGE_ID);
    let prepared = success_json(repository.run(&[
        "prepare-approval",
        "--canonical",
        KNOWLEDGE_ID,
        "--revision",
        &revision,
        "--reviewer",
        OWNER_ID,
    ]));
    let request = repository.request("canonical-approval.json", &prepared);
    success_json(repository.run(&["approve", request.to_str().unwrap()]));
    repository.git(&["add", "methexis/approvals"]);
    repository.git(&["commit", "-m", "integrate canonical approval"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let checkpoint_request = repository.request(
        "canonical-checkpoint.json",
        &json!({
            "schema": "methexis.checkpoint-request/v1alpha1",
            "roots": [KNOWLEDGE_ID]
        }),
    );
    let created =
        success_json(repository.run(&["create-checkpoint", checkpoint_request.to_str().unwrap()]));
    let create_output = repository.request("canonical-create-output.json", &created);
    let activation =
        success_json(repository.run(&["prepare-activation", create_output.to_str().unwrap()]));
    let activation_request = repository.request("canonical-activation.json", &activation);
    let proposed =
        success_json(repository.run(&["propose-activation", activation_request.to_str().unwrap()]));
    assert_eq!(proposed["status"], "written");
    assert!(
        !repository
            .path
            .join(format!("methexis/review-projections/{KNOWLEDGE_ID}.md"))
            .exists()
    );
}
