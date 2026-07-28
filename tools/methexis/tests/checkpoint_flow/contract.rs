//! Agent fixtures, trusted approval, immutable Checkpoint, and authority transition.

use std::{fs, path::Path};

use serde_json::{Value, json};

use super::support::*;

// 각 checkpoint 예제 요청으로 실제 CLI 명령을 실행하고 성공·실패 JSON 전체를 golden과 비교한다.
// 요청·응답 schema가 바뀌었는데 agent용 예제가 뒤처지는 문제를 이 비교로 잡는다.
#[test]
fn agent_contract_fixtures_are_complete_and_current() {
    let repository = GitRepository::approved();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/checkpoint-contract");

    for (command, request, expected) in [
        (
            "create-checkpoint",
            "checkpoint-request.json",
            "checkpoint-success.json",
        ),
        (
            "propose-activation",
            "activation-request.json",
            "activation-success.json",
        ),
    ] {
        let actual =
            success_json(repository.run(&[command, examples.join(request).to_str().unwrap()]));
        let expected: Value =
            serde_json::from_slice(&fs::read(examples.join(expected)).unwrap()).unwrap();
        assert_eq!(actual, expected);
    }

    let actual = failure_json(repository.run(&[
        "create-checkpoint",
        examples.join("unknown-root-request.json").to_str().unwrap(),
    ]));
    let expected: Value =
        serde_json::from_slice(&fs::read(examples.join("unknown-root.json")).unwrap()).unwrap();
    assert_eq!(actual, expected);
}

// 같은 checkpoint 초안 생성은 중복 없이 재사용되고 activation 제안만 한 상태에서는 inactive다.
// 승인된 지식과 fresh decision 근거를 신뢰 브랜치에 통합한 뒤에만 active가 되며, 외부 graft로
// Git 이력을 꾸며도 이 권한 판정이 바뀌지 않는지 확인한다.
#[test]
fn trusted_activation_becomes_active_when_decision_sources_are_fresh() {
    let repository = GitRepository::approved();
    let create_request = repository.request("checkpoint.json", &checkpoint_request());
    let created =
        success_json(repository.run(&["create-checkpoint", create_request.to_str().unwrap()]));
    assert_eq!(created["authority"], "draft_proposal");
    assert_eq!(created["status"], "written");
    let repeated =
        success_json(repository.run(&["create-checkpoint", create_request.to_str().unwrap()]));
    assert_eq!(repeated["status"], "unchanged");
    assert_eq!(repeated["checkpoint_id"], created["checkpoint_id"]);

    let activation_request = repository.request(
        "activation.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": created["checkpoint_id"],
            "checkpoint_hash": created["hash"]
        }),
    );
    let activation =
        success_json(repository.run(&["propose-activation", activation_request.to_str().unwrap()]));
    assert_eq!(activation["status"], "written");

    let before = success_json(repository.run(&["check"]));
    assert_eq!(before["checkpoint"], "inactive");
    assert_eq!(before["units"][0]["effective_approval"], "approved");
    assert_eq!(before["units"][0]["eligibility"], "inactive");

    repository.git(&[
        "add",
        "methexis/checkpoints",
        "methexis/active-checkpoint.yaml",
    ]);
    repository.git(&["commit", "-m", "activate fixture checkpoint"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let after = success_json(repository.run(&["check"]));
    assert_eq!(after["checkpoint"], "active");
    assert_eq!(after["units"][0]["effective_approval"], "approved");
    assert_eq!(after["units"][0]["eligibility"], "active");
    assert_eq!(
        after["units"][0]["eligibility_evidence"],
        json!(["decision_revision_match:tui.fixture"])
    );
    let trusted_commit = repository.git(&["rev-parse", "develop"]);
    let trusted_commit = String::from_utf8(trusted_commit.stdout).unwrap();
    assert_eq!(after["trusted_commit"], trusted_commit.trim());

    let info = repository.path.join(".git/info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("grafts"), format!("{}\n", trusted_commit.trim())).unwrap();
    let graft_isolated = success_json(repository.run(&["check"]));
    assert_eq!(graft_isolated["checkpoint"], "active");
}

// checkpoint가 trusted integration에서 활성화된 뒤 참조한 code 바이트가 바뀌면 현재 실행 입력과
// 활성화 시점 입력이 달라진다. 지식 승인과 trusted commit은 보존하되 checkpoint는 degraded,
// 해당 지식은 stale로 낮춰 사용을 막는다.
#[test]
fn trusted_code_activation_degrades_without_losing_approval_on_byte_drift() {
    let repository = GitRepository::code_approved();
    repository.integrate_active_checkpoint();

    let active = success_json(repository.run(&["check"]));
    assert_eq!(active["checkpoint"], "active");
    assert_eq!(active["units"][0]["effective_approval"], "approved");
    assert_eq!(active["units"][0]["eligibility"], "active");
    let trusted_commit = active["trusted_commit"].clone();

    fs::write(
        repository.path.join("methexis/code-source.txt"),
        b"drifted\n",
    )
    .unwrap();
    let degraded = success_json(repository.run(&["check"]));

    assert_eq!(degraded["checkpoint"], "degraded");
    assert_eq!(degraded["trusted_commit"], trusted_commit);
    assert_eq!(degraded["units"][0]["effective_approval"], "approved");
    assert_eq!(degraded["units"][0]["eligibility"], "stale");
    assert_eq!(
        degraded["units"][0]["eligibility_evidence"],
        json!(["code_hash_mismatch:tui.code-fixture"])
    );
}
