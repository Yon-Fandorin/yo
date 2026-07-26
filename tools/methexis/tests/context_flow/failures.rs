use std::{fs, os::unix::fs::symlink};

use serde_json::json;

use super::{
    repository::GitRepository,
    support::{
        active_repository, candidate_request, direct_request, raw_resolve, resolve, resolve_failure,
    },
};

#[test]
fn unresolved_direct_anchor_and_required_over_budget_fail_without_stdout() {
    let repository = active_repository();
    let unresolved = direct_request(&repository, "symbol", "yo::missing", 8_000);
    let output = raw_resolve(&repository, &unresolved);
    assert!(output.stdout.is_empty());
    let failure: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(failure["error"]["code"], "explicit_anchor_unresolved");

    let over_budget = direct_request(&repository, "knowledge_id", "tui.context.large", 1);
    let failure = resolve_failure(&repository, &over_budget);
    assert_eq!(failure["error"]["code"], "required_budget_exceeded");
}

#[test]
fn stale_required_knowledge_fails_and_stale_optional_candidate_is_omitted() {
    let repository = GitRepository::code_approved();
    repository.integrate_active_checkpoint();
    fs::write(
        repository.path.join("methexis/code-source.txt"),
        "changed after activation\n",
    )
    .unwrap();

    let required = direct_request(&repository, "knowledge_id", "tui.relocated", 8_000);
    let failure = resolve_failure(&repository, &required);
    assert_eq!(failure["error"]["code"], "required_knowledge_blocked");

    let optional = candidate_request(&repository, &[("tui.relocated", 100)], 8_000, false);
    let result = resolve(&repository, &optional);
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            repository
                .path
                .join(result["manifest"]["path"].as_str().unwrap()),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(result["affected_ids"], json!([]));
    assert_eq!(
        manifest["plan"]["candidate_decisions"][0]["reason"],
        "bundle_stale"
    );
}

#[test]
fn corrupted_existing_build_fails_and_quarantines_new_output() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    let created = resolve(&repository, &request);
    let manifest = repository
        .path
        .join(created["manifest"]["path"].as_str().unwrap());
    fs::write(&manifest, b"corrupted\n").unwrap();

    let failure = resolve_failure(&repository, &request);

    assert_eq!(failure["error"]["code"], "context_build_collision");
    assert_eq!(fs::read(&manifest).unwrap(), b"corrupted\n");
    assert!(
        fs::read_dir(repository.path.join(".local-exclude/methexis/quarantine"))
            .unwrap()
            .next()
            .is_some()
    );
}

#[test]
fn existing_build_with_an_unexpected_file_is_a_collision() {
    let repository = active_repository();
    let request = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    let created = resolve(&repository, &request);
    let build = repository
        .path
        .join(created["context"]["path"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_owned();
    fs::write(build.join("unexpected"), b"not part of the artifact set\n").unwrap();

    let failure = resolve_failure(&repository, &request);

    assert_eq!(failure["error"]["code"], "context_build_collision");
}

#[test]
fn symlinked_candidate_and_build_paths_fail_closed() {
    let repository = active_repository();
    let outside = repository.path.join("outside.json");
    fs::write(&outside, b"{}\n").unwrap();
    let candidate_link = repository.path.join(".local-exclude/candidate-link.json");
    symlink(&outside, &candidate_link).unwrap();
    let request = repository.request(
        "symlink-candidate.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "candidates": {
                "path": ".local-exclude/candidate-link.json",
                "hash": format!("sha256:{}", "0".repeat(64))
            },
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": 8000
        }),
    );
    let failure = resolve_failure(&repository, &request);
    assert_eq!(failure["error"]["code"], "candidate_path_invalid");

    let direct = direct_request(&repository, "knowledge_id", "tui.context.base", 8_000);
    let created = resolve(&repository, &direct);
    let build = repository
        .path
        .join(created["context"]["path"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_owned();
    fs::remove_dir_all(&build).unwrap();
    let outside_directory = repository.path.join("outside-build");
    fs::create_dir(&outside_directory).unwrap();
    symlink(&outside_directory, &build).unwrap();

    let failure = resolve_failure(&repository, &direct);
    assert_eq!(failure["error"]["code"], "context_path_symlink");
}

#[test]
fn request_and_candidate_contract_failures_are_structured() {
    let repository = active_repository();
    let unsupported = repository.request(
        "unsupported-tokenizer.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "anchors": [{"kind": "knowledge_id", "value": "tui.context.base"}],
            "tokenizer_profile": "character-estimate/v1",
            "max_tokens": 8000
        }),
    );
    assert_eq!(
        resolve_failure(&repository, &unsupported)["error"]["code"],
        "unsupported_tokenizer_profile"
    );

    let empty = repository.request(
        "empty.json",
        &json!({
            "schema": "methexis.context-request/v1alpha1",
            "tokenizer_profile": "o200k_base/v1",
            "max_tokens": 8000
        }),
    );
    assert_eq!(
        resolve_failure(&repository, &empty)["error"]["code"],
        "empty_context_request"
    );
}
