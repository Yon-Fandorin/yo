use std::path;

use serde_json::json;

use super::{alpha2, alpha3, alpha4};
use crate::validation_summary::verify_review_input;

// 패킷 준비 단계가 runner 내부 이름과 exact candidate를 묶어 늦은 gate 실패를 막는다.
#[test]
fn review_input_binds_name_and_exact_candidate_before_publication() {
    let candidate = "a".repeat(40);
    let bytes = alpha2("yo-cli", &candidate);
    let repository = path::Path::new(".");

    verify_review_input(repository, &bytes, "yo-cli", &candidate).unwrap();
    assert!(
        verify_review_input(repository, &bytes, "yo-cli-tests", &candidate)
            .unwrap_err()
            .contains("does not match requested evidence name")
    );
    assert!(
        verify_review_input(repository, &bytes, "yo-cli", &"b".repeat(40))
            .unwrap_err()
            .contains("does not match candidate")
    );
}

// 실행 정체성을 증명하지 못하는 dirty, reused, 빈 argv 기록은 리뷰 입력이 될 수 없다.
#[test]
fn review_input_rejects_dirty_reused_or_unidentified_execution() {
    let candidate = "a".repeat(40);
    let repository = path::Path::new(".");
    for (field, replacement, message) in [
        ("worktree_state", json!("dirty"), "worktree_state"),
        ("reused", json!(true), "reused:false"),
        ("command_argv_count", json!(0), "greater than zero"),
    ] {
        let mut summary: serde_json::Value =
            serde_json::from_slice(&alpha2("yo-cli", &candidate)).unwrap();
        summary[field] = replacement;
        let bytes = serde_json::to_vec(&summary).unwrap();
        assert!(
            verify_review_input(repository, &bytes, "yo-cli", &candidate)
                .unwrap_err()
                .contains(message)
        );
    }
}

// v1alpha3 review evidence는 기존 실행 정체성에 더해 typed reuse context를
// packet 발행 전에 검사하고 frozen v1alpha2 의미를 바꾸지 않는다.
#[test]
fn alpha3_review_input_requires_a_closed_reuse_context() {
    let candidate = "a".repeat(40);
    let repository = path::Path::new(".");
    let bytes = alpha3("yo-cli", &candidate);
    verify_review_input(repository, &bytes, "yo-cli", &candidate).unwrap();

    let mut summary: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    summary["reuse_context"]["external_state"] = json!("remote-provider");
    let error = verify_review_input(
        repository,
        &serde_json::to_vec(&summary).unwrap(),
        "yo-cli",
        &candidate,
    )
    .unwrap_err();
    assert!(error.contains("none-declared"));
}

// leased v1alpha4 evidence is accepted only when both the inherited execution identity
// and its closed, non-waiting resource observation are intact.
#[test]
fn alpha4_review_input_requires_closed_resource_lease() {
    let candidate = "a".repeat(40);
    let repository = path::Path::new(".");
    let bytes = alpha4("yo-cli", &candidate);
    verify_review_input(repository, &bytes, "yo-cli", &candidate).unwrap();

    let mut summary: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    summary["resource_lease"]["wait_attempts"] = json!(1);
    assert!(
        verify_review_input(
            repository,
            &serde_json::to_vec(&summary).unwrap(),
            "yo-cli",
            &candidate,
        )
        .unwrap_err()
        .contains("resource lease")
    );
}
