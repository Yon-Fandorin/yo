mod command;
mod reuse;
mod schemas;

use std::env;

use serde_json::json;

use super::{argv_hash, current_toolchain_hash};

fn alpha2(name: &str, candidate: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": "yo.validation-run-summary/v1alpha2",
        "name": name,
        "status": "passed",
        "exit_code": 0,
        "elapsed_seconds": 1,
        "log_bytes": 1,
        "log_path": ".local-exclude/test.log",
        "log_hash": format!("sha256:{}", "1".repeat(64)),
        "head_commit": candidate,
        "worktree_state": "clean",
        "command_argv_count": 2,
        "command_argv_hash": argv_hash(&["cargo".to_owned(), "test".to_owned()]),
        "reused": false,
        "reuse_policy": "reviewed-descendant/v1"
    }))
    .unwrap()
}

fn alpha3(name: &str, candidate: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": "yo.validation-run-summary/v1alpha3",
        "name": name,
        "status": "passed",
        "exit_code": 0,
        "elapsed_seconds": 1,
        "log_bytes": 1,
        "log_path": ".local-exclude/test.log",
        "log_hash": format!("sha256:{}", "1".repeat(64)),
        "head_commit": candidate,
        "worktree_state": "clean",
        "command_argv_count": 2,
        "command_argv_hash": argv_hash(&["cargo".to_owned(), "test".to_owned()]),
        "reused": false,
        "reuse_policy": "reviewed-descendant-context/v1",
        "reuse_context": {
            "schema": "yo.validation-reuse-context/v1alpha1",
            "platform_os": env::consts::OS,
            "platform_arch": env::consts::ARCH,
            "toolchain_hash": current_toolchain_hash().unwrap(),
            "external_state": "none-declared"
        }
    }))
    .unwrap()
}

fn alpha4(name: &str, candidate: &str) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_slice(&alpha3(name, candidate)).unwrap();
    value["schema"] = json!("yo.validation-run-summary/v1alpha4");
    value["resource_lease"] = json!({
        "schema": "yo.validation-resource-lease/v1alpha1",
        "class": "cargo-heavy",
        "key": "cargo-heavy",
        "status": "acquired",
        "wait_attempts": 0
    });
    serde_json::to_vec(&value).unwrap()
}
