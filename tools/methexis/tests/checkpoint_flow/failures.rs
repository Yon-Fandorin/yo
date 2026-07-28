//! Checkpoint request and active compare-and-swap failures.

use std::fs;

use serde_json::json;

use super::support::*;

// 알 수 없는 root나 잘못된 checkpoint hash는 active 출력을 전혀 게시하지 않고 거부한다.
#[test]
fn unknown_root_and_wrong_checkpoint_hash_publish_nothing() {
    let repository = GitRepository::approved();
    let unknown = repository.request(
        "unknown.json",
        &json!({
            "schema": "methexis.checkpoint-request/v1alpha1",
            "roots": ["unknown.root"]
        }),
    );
    let failure = failure_json(repository.run(&["create-checkpoint", unknown.to_str().unwrap()]));
    assert_eq!(failure["error"]["code"], "unknown_checkpoint_root");
    assert!(!repository.path.join("methexis/checkpoints").exists());

    let create = repository.request("checkpoint.json", &checkpoint_request());
    let created = success_json(repository.run(&["create-checkpoint", create.to_str().unwrap()]));
    let bad_activation = repository.request(
        "bad-activation.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": created["checkpoint_id"],
            "checkpoint_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        }),
    );
    let failure =
        failure_json(repository.run(&["propose-activation", bad_activation.to_str().unwrap()]));
    assert_eq!(failure["error"]["code"], "checkpoint_mismatch");
    assert!(
        !repository
            .path
            .join("methexis/active-checkpoint.yaml")
            .exists()
    );
}

// 미승인 root와 이동된 trust anchor는 권한을 추측하지 않고 fail-closed로 거부한다.
#[test]
fn unapproved_root_and_moved_trust_anchor_fail_closed() {
    let repository = GitRepository::foundation();
    let create = repository.request("checkpoint.json", &checkpoint_request());
    let failure = failure_json(repository.run(&["create-checkpoint", create.to_str().unwrap()]));
    assert_eq!(failure["error"]["code"], "trusted_approval_missing");
    assert!(!repository.path.join("methexis/checkpoints").exists());

    let repository = GitRepository::approved();
    let create = repository.request("checkpoint.json", &checkpoint_request());
    let created = success_json(repository.run(&["create-checkpoint", create.to_str().unwrap()]));
    repository.git(&[
        "commit",
        "--allow-empty",
        "-m",
        "advance trusted integration",
    ]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    let activation = repository.request(
        "activation.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": created["checkpoint_id"],
            "checkpoint_hash": created["hash"]
        }),
    );
    let failure =
        failure_json(repository.run(&["propose-activation", activation.to_str().unwrap()]));
    assert_eq!(failure["error"]["code"], "checkpoint_trust_mismatch");
    assert_eq!(
        failure["trusted_commit"],
        String::from_utf8(repository.git(&["rev-parse", "develop"]).stdout)
            .unwrap()
            .trim()
    );
    assert!(
        !repository
            .path
            .join("methexis/active-checkpoint.yaml")
            .exists()
    );
}

// 손상된 checkpoint는 active 출력을 남기지 않고 구조화된 실패로 거부한다.
#[test]
fn damaged_checkpoint_is_rejected_without_active_output() {
    let repository = GitRepository::approved();
    let create = repository.request("checkpoint.json", &checkpoint_request());
    let created = success_json(repository.run(&["create-checkpoint", create.to_str().unwrap()]));
    let checkpoint = repository.path.join(created["path"].as_str().unwrap());
    fs::write(checkpoint, b"not: [valid\n").unwrap();
    let activation = repository.request(
        "activation.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": created["checkpoint_id"],
            "checkpoint_hash": created["hash"]
        }),
    );
    let failure =
        failure_json(repository.run(&["propose-activation", activation.to_str().unwrap()]));
    assert_eq!(failure["error"]["code"], "invalid_checkpoint");
    assert!(
        !repository
            .path
            .join("methexis/active-checkpoint.yaml")
            .exists()
    );
}

// 호출자가 Git 환경 변수나 같은 이름의 replacement ref를 주입해도 신뢰 기준을 바꿀 수 없어야 한다.
// checkpoint는 저장소가 정한 실제 authority ref만 읽어 권한을 판정한다.
#[test]
fn caller_git_environment_and_replacement_refs_cannot_change_authority() {
    let repository = GitRepository::approved();
    let other = GitRepository::foundation();
    let create = repository.request("checkpoint-env.json", &checkpoint_request());
    let foreign_git_dir = other.path.join(".git");
    let created = success_json(repository.run_with_env(
        &["create-checkpoint", create.to_str().unwrap()],
        &[("GIT_DIR", &foreign_git_dir), ("PATH", &other.path)],
    ));
    assert_eq!(created["affected_ids"][0], KNOWLEDGE_ID);

    std::fs::remove_file(repository.path.join(created["path"].as_str().unwrap())).unwrap();
    let trusted = String::from_utf8(repository.git(&["rev-parse", "develop"]).stdout).unwrap();
    let parent = String::from_utf8(repository.git(&["rev-parse", "develop^"]).stdout).unwrap();
    repository.git(&["replace", trusted.trim(), parent.trim()]);
    let created = success_json(repository.run(&["create-checkpoint", create.to_str().unwrap()]));
    assert_eq!(created["affected_ids"][0], KNOWLEDGE_ID);
}

// 새 지식이 이전 지식을 대체하도록 승인됐다면 두 버전을 한 context에 동시에 넣어서는 안 된다.
// checkpoint가 replacement와 superseded 지식을 함께 선택하는 모순을 거부한다.
#[test]
fn checkpoint_cannot_select_a_replacement_with_its_superseded_unit() {
    let repository = GitRepository::foundation();
    let replacement = repository
        .path
        .join("methexis/knowledge/first-location/replacement.md");
    fs::write(
        replacement,
        br#"---
schema: methexis.knowledge/v1alpha1
id: tui.replacement
kind: rule
owner: tui-architecture
sources:
  - id: tui.fixture
    revision: sha256:3d3ff9057aadcbf2f44300bce0f97c5c84dc3c59a1a76e09eb012b299892f130
relations:
  supersedes:
    - tui.relocated
---
## Statement

Replacement meaning.
"#,
    )
    .unwrap();
    repository.git(&["add", "methexis"]);
    repository.git(&["commit", "-m", "add replacement"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    repository.approve_units(&[KNOWLEDGE_ID, "tui.replacement"]);
    repository.git(&["add", "methexis"]);
    repository.git(&["commit", "-m", "approve replacement pair"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let request = repository.request(
        "checkpoint-superseded.json",
        &json!({
            "schema": "methexis.checkpoint-request/v1alpha1",
            "roots": [KNOWLEDGE_ID, "tui.replacement"]
        }),
    );
    let failure = failure_json(repository.run(&["create-checkpoint", request.to_str().unwrap()]));
    assert_eq!(failure["error"]["code"], "superseded_units_co_selected");
    assert!(!repository.path.join("methexis/checkpoints").exists());
}
