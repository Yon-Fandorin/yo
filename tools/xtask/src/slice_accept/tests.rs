use super::{
    ACCEPT_REQUEST_SCHEMA, ACCEPT_REQUEST_SCHEMA_V1_ALPHA3, AcceptRequest, COMMIT_GIT_HOOKS, Push,
    accept_result,
};

// 동결된 accept result에는 새 verification 필드를 보태지 않고, 새 v1alpha3
// 결과에서만 실제 선택된 hook 검증 방식을 공개합니다.
#[test]
fn accept_result_keeps_legacy_fields_frozen() {
    let mut request = AcceptRequest {
        schema: ACCEPT_REQUEST_SCHEMA.to_owned(),
        slice: "example".to_owned(),
        gate_request_path: "gate.json".to_owned(),
        gate_request_hash: format!("sha256:{}", "a".repeat(64)),
        message_source_path: "message.txt".to_owned(),
        message_source_hash: format!("sha256:{}", "b".repeat(64)),
        message_output_path: "message.out".to_owned(),
        close_prepare_request_path: "close.json".to_owned(),
        close_prepare_request_hash: format!("sha256:{}", "c".repeat(64)),
        close_plan_path: "plan.json".to_owned(),
        push: Some(Push {
            remote: "origin".to_owned(),
            reference: "refs/heads/develop".to_owned(),
        }),
        commit_verification: None,
        approval_scope: Some("legacy".to_owned()),
        effect_scope: None,
    };
    let legacy = accept_result(
        &request,
        "candidate",
        "accepted",
        "develop",
        COMMIT_GIT_HOOKS,
    );
    assert!(legacy.get("commit_verification").is_none());

    request.schema = ACCEPT_REQUEST_SCHEMA_V1_ALPHA3.to_owned();
    let current = accept_result(
        &request,
        "candidate",
        "accepted",
        "develop",
        COMMIT_GIT_HOOKS,
    );
    assert_eq!(current["commit_verification"], COMMIT_GIT_HOOKS);
}
