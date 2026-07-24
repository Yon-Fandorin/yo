//! Agent fixtures, trusted approval, immutable Checkpoint, and authority transition.

use std::{fs, path::Path};

use serde_json::{Value, json};

use super::support::*;

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

#[test]
fn trusted_activation_remains_closed_until_source_validation_exists() {
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
    assert_eq!(after["checkpoint"], "pending_source_validation");
    assert_eq!(after["units"][0]["effective_approval"], "approved");
    assert_eq!(after["units"][0]["eligibility"], "inactive");
    let trusted_commit = repository.git(&["rev-parse", "develop"]);
    let trusted_commit = String::from_utf8(trusted_commit.stdout).unwrap();
    assert_eq!(after["trusted_commit"], trusted_commit.trim());

    let info = repository.path.join(".git/info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("grafts"), format!("{}\n", trusted_commit.trim())).unwrap();
    let graft_isolated = success_json(repository.run(&["check"]));
    assert_eq!(graft_isolated["checkpoint"], "pending_source_validation");
}
