//! Active-record compare-and-swap replacement.

use serde_json::json;

use super::support::*;

// active checkpoint 교체는 현재 active record의 정확한 hash를 제시할 때만 허용한다.
#[test]
fn activation_replacement_requires_the_exact_active_record_hash() {
    let repository = GitRepository::approved();
    let first_create = repository.request("checkpoint-first.json", &checkpoint_request());
    let first =
        success_json(repository.run(&["create-checkpoint", first_create.to_str().unwrap()]));
    let first_activation = repository.request(
        "activation-first.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": first["checkpoint_id"],
            "checkpoint_hash": first["hash"]
        }),
    );
    let active =
        success_json(repository.run(&["propose-activation", first_activation.to_str().unwrap()]));
    let original = std::fs::read(repository.path.join("methexis/active-checkpoint.yaml")).unwrap();

    repository.git(&[
        "commit",
        "--allow-empty",
        "-m",
        "advance trusted integration",
    ]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    let second_create = repository.request("checkpoint-second.json", &checkpoint_request());
    let second =
        success_json(repository.run(&["create-checkpoint", second_create.to_str().unwrap()]));
    let malformed = repository.request(
        "activation-malformed-predecessor.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": second["checkpoint_id"],
            "checkpoint_hash": second["hash"],
            "replace_active_hash": "not-a-sha256"
        }),
    );
    let malformed =
        failure_json(repository.run(&["propose-activation", malformed.to_str().unwrap()]));
    assert_eq!(malformed["error"]["code"], "invalid_activation_request");

    let conflicting = repository.request(
        "activation-conflicting.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": second["checkpoint_id"],
            "checkpoint_hash": second["hash"]
        }),
    );
    let failure =
        failure_json(repository.run(&["propose-activation", conflicting.to_str().unwrap()]));
    assert_eq!(failure["error"]["code"], "activation_conflict");
    assert_eq!(
        std::fs::read(repository.path.join("methexis/active-checkpoint.yaml")).unwrap(),
        original
    );

    let replacement = repository.request(
        "activation-replacement.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": second["checkpoint_id"],
            "checkpoint_hash": second["hash"],
            "replace_active_hash": active["hash"]
        }),
    );
    let replaced =
        success_json(repository.run(&["propose-activation", replacement.to_str().unwrap()]));
    assert_eq!(replaced["status"], "written");
    assert_eq!(replaced["checkpoint_id"], second["checkpoint_id"]);
    let active_record: serde_json::Value = serde_norway::from_slice(
        &std::fs::read(repository.path.join("methexis/active-checkpoint.yaml")).unwrap(),
    )
    .unwrap();
    // persisted active lineage가 요청의 정확한 CAS 전임자를 보존해야 staged 검사가 재현할 수 있다.
    assert_eq!(active_record["replaces_active_hash"], active["hash"]);
}

// 성공 출력은 pinned trusted snapshot의 active Checkpoint와 비교한 변화만 싣는다.
// 그대로 남은 unit의 전체 식별자 목록은 반복하지 않고 count만 제공하며, create와
// propose가 같은 candidate delta를 보고해야 한다.
#[test]
fn checkpoint_success_reports_only_the_reproducible_active_delta() {
    const SECOND_ID: &str = "tui.secondary";
    let repository = GitRepository::foundation();
    let second = repository
        .path
        .join("methexis/knowledge/first-location/secondary.md");
    std::fs::write(
        second,
        br#"---
schema: methexis.knowledge/v1alpha1
id: tui.secondary
kind: rule
owner: tui-architecture
sources:
  - id: tui.fixture
    revision: sha256:3d3ff9057aadcbf2f44300bce0f97c5c84dc3c59a1a76e09eb012b299892f130
---
## Statement

Secondary fixture knowledge remains independently selectable.
"#,
    )
    .unwrap();
    repository.git(&["add", "methexis/knowledge"]);
    repository.git(&["commit", "-m", "add secondary fixture knowledge"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);
    repository.approve_units(&[KNOWLEDGE_ID, SECOND_ID]);
    repository.git(&["add", "methexis"]);
    repository.git(&["commit", "-m", "approve fixture knowledge"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let first_create = repository.request(
        "checkpoint-delta-first.json",
        &json!({
            "schema": "methexis.checkpoint-request/v1alpha1",
            "roots": [KNOWLEDGE_ID, SECOND_ID]
        }),
    );
    let first =
        success_json(repository.run(&["create-checkpoint", first_create.to_str().unwrap()]));
    let first_activation = repository.request(
        "activation-delta-first.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": first["checkpoint_id"],
            "checkpoint_hash": first["hash"]
        }),
    );
    let active =
        success_json(repository.run(&["propose-activation", first_activation.to_str().unwrap()]));
    repository.git(&[
        "add",
        "methexis/checkpoints",
        "methexis/active-checkpoint.yaml",
    ]);
    repository.git(&["commit", "-m", "activate both fixture units"]);
    repository.git(&["branch", "-f", "develop", "HEAD"]);

    let second_create = repository.request(
        "checkpoint-delta-second.json",
        &json!({
            "schema": "methexis.checkpoint-request/v1alpha1",
            "roots": [KNOWLEDGE_ID]
        }),
    );
    let second =
        success_json(repository.run(&["create-checkpoint", second_create.to_str().unwrap()]));
    let delta = &second["checkpoint_delta"];
    assert_eq!(
        delta["baseline"],
        json!({
            "checkpoint_id": first["checkpoint_id"],
            "checkpoint_hash": first["hash"]
        })
    );
    assert_eq!(delta["candidate"]["checkpoint_id"], second["checkpoint_id"]);
    assert_eq!(delta["candidate"]["checkpoint_hash"], second["hash"]);
    assert_eq!(delta["candidate"]["artifact_path"], second["path"]);
    assert_eq!(delta["candidate_unit_count"], 1);
    assert_eq!(delta["unchanged_unit_count"], 1);
    assert_eq!(
        delta["unit_changes"],
        json!([{
            "id": SECOND_ID,
            "before_revision": repository.revision_for(SECOND_ID),
            "after_revision": null
        }])
    );
    assert_eq!(
        delta["root_changes"],
        json!([{
            "root": SECOND_ID,
            "before_present": true,
            "after_present": false
        }])
    );

    let second_activation = repository.request(
        "activation-delta-second.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": second["checkpoint_id"],
            "checkpoint_hash": second["hash"],
            "replace_active_hash": active["hash"]
        }),
    );
    let proposal =
        success_json(repository.run(&["propose-activation", second_activation.to_str().unwrap()]));
    assert_eq!(proposal["checkpoint_delta"], second["checkpoint_delta"]);
}
