//! Reproduction of Checkpoint bytes from their claimed trusted Git commit.

use std::{fmt::Write, fs};

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::support::*;

#[derive(Deserialize, Serialize)]
struct Checkpoint {
    schema: String,
    checkpoint_id: String,
    trusted_commit: String,
    source_status: String,
    roots: Vec<String>,
    units: Vec<Unit>,
}

#[derive(Deserialize, Serialize)]
struct Unit {
    id: String,
    revision: String,
    reasons: Vec<String>,
}

#[derive(Serialize)]
struct Identity<'a> {
    schema: &'a str,
    trusted_commit: &'a str,
    source_status: &'a str,
    roots: &'a [String],
    units: &'a [Unit],
}

#[test]
fn self_consistent_but_unreproducible_checkpoint_is_rejected() {
    let repository = GitRepository::approved();
    let create = repository.request("checkpoint.json", &checkpoint_request());
    let created = success_json(repository.run(&["create-checkpoint", create.to_str().unwrap()]));
    let original = repository.path.join(created["path"].as_str().unwrap());
    let mut checkpoint: Checkpoint =
        serde_norway::from_slice(&fs::read(original).unwrap()).unwrap();
    checkpoint.units[0]
        .reasons
        .push("root:unclaimed".to_owned());
    checkpoint.units[0].reasons.sort();
    checkpoint.checkpoint_id = hash_json(&Identity {
        schema: &checkpoint.schema,
        trusted_commit: &checkpoint.trusted_commit,
        source_status: &checkpoint.source_status,
        roots: &checkpoint.roots,
        units: &checkpoint.units,
    });
    let bytes = serde_norway::to_string(&checkpoint).unwrap().into_bytes();
    let path = repository.path.join("methexis/checkpoints").join(format!(
        "{}.yaml",
        checkpoint.checkpoint_id.strip_prefix("sha256:").unwrap()
    ));
    fs::write(path, &bytes).unwrap();
    let activation = repository.request(
        "activation-unreproducible.json",
        &json!({
            "schema": "methexis.activation-request/v1alpha1",
            "checkpoint_id": checkpoint.checkpoint_id,
            "checkpoint_hash": hash_bytes(&bytes)
        }),
    );
    let failure =
        failure_json(repository.run(&["propose-activation", activation.to_str().unwrap()]));
    assert_eq!(failure["error"]["code"], "checkpoint_lineage_mismatch");
    assert!(
        !repository
            .path
            .join("methexis/active-checkpoint.yaml")
            .exists()
    );
}

fn hash_json(value: &impl Serialize) -> String {
    hash_bytes(&serde_json::to_vec(value).unwrap())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hash = "sha256:".to_owned();
    for byte in Sha256::digest(bytes) {
        write!(hash, "{byte:02x}").unwrap();
    }
    hash
}
