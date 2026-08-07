//! Discriminating fail-closed behavior of the prepare commands.

use serde_json::json;

use super::support::*;

// owners foundation에 없는 reviewer는 approve까지 가지 않고 준비 시점에 unknown_reviewer로 닫힌다.
#[test]
fn unknown_reviewer_fails_at_preparation_time() {
    let repository = GitRepository::foundation();
    let revision = repository.revision_for(KNOWLEDGE_ID);
    let projection = repository.project(KNOWLEDGE_ID, &revision);
    let manifest = repository.build_manifest(
        KNOWLEDGE_ID,
        &revision,
        projection["hash"].as_str().unwrap(),
    );

    let failure =
        failure_json(repository.run(&["prepare-approval", &manifest, "--reviewer", "nobody"]));

    assert_eq!(failure["ok"], false);
    assert_eq!(failure["operation"], "prepare_approval");
    assert_eq!(failure["error"]["code"], "unknown_reviewer");
    assert_eq!(failure["error"]["affected_ids"], json!([KNOWLEDGE_ID]));
}

// 기존 승인 기록이 없는 단위에 --replace-current를 요청하면 CAS predecessor를 지을 수 없으므로
// 닫힌다.
#[test]
fn replace_current_without_an_existing_record_fails_closed() {
    let repository = GitRepository::foundation();
    let revision = repository.revision_for(KNOWLEDGE_ID);
    let projection = repository.project(KNOWLEDGE_ID, &revision);
    let manifest = repository.build_manifest(
        KNOWLEDGE_ID,
        &revision,
        projection["hash"].as_str().unwrap(),
    );

    let failure = failure_json(repository.run(&[
        "prepare-approval",
        &manifest,
        "--reviewer",
        OWNER_ID,
        "--replace-current",
    ]));

    assert_eq!(failure["ok"], false);
    assert_eq!(failure["error"]["code"], "approval_unreadable");
}

// 활성 Checkpoint가 아직 없으면 prepare-checkpoint는 roots를 추론할 수 없으므로 구조화된 진단으로
// 닫힌다.
#[test]
fn prepare_checkpoint_without_an_active_checkpoint_fails_closed() {
    let repository = GitRepository::approved();

    let failure = failure_json(repository.run(&["prepare-checkpoint"]));

    assert_eq!(failure["ok"], false);
    assert_eq!(failure["operation"], "prepare_checkpoint");
    assert_eq!(failure["error"]["code"], "no_active_checkpoint");
}

// create-checkpoint가 아닌 다른 operation의 성공 출력을 넣으면 prepare-activation은 닫힌다.
#[test]
fn prepare_activation_rejects_non_create_operation_output() {
    let repository = GitRepository::approved();
    let output = repository.request(
        "not-create.json",
        &json!({
            "schema": "methexis.operation/v1alpha1",
            "ok": true,
            "operation": "propose_activation",
            "checkpoint_id": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        }),
    );

    let failure = failure_json(repository.run(&["prepare-activation", output.to_str().unwrap()]));

    assert_eq!(failure["ok"], false);
    assert_eq!(failure["operation"], "prepare_activation");
    assert_eq!(failure["error"]["code"], "invalid_create_output");
}

// --reviewer 없이 manifest만 주면 인자 오류를 구조화된 진단으로 닫고 아무 파일도 읽지 않는다.
#[test]
fn prepare_approval_requires_an_explicit_reviewer() {
    let repository = GitRepository::foundation();

    let failure = failure_json(repository.run(&["prepare-approval", "manifest.json"]));

    assert_eq!(failure["schema"], "methexis.error/v1alpha1");
    assert_eq!(failure["error"]["code"], "invalid_prepare_arguments");
}
