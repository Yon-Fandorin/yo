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
}
