use crate::{slice_close::prepare::build_metrics_for_test, slice_gate};

fn gate(environments: Vec<String>) -> slice_gate::ReadyGate {
    slice_gate::ReadyGate {
        slice: "sample".to_owned(),
        candidate_commit: "1111111111111111111111111111111111111111".to_owned(),
        diff_hash: "sha256:diff".to_owned(),
        validation: vec![slice_gate::ReadyValidation {
            name: "xtask".to_owned(),
            argv: vec![
                "cargo".to_owned(),
                "test".to_owned(),
                "-p".to_owned(),
                "xtask".to_owned(),
            ],
            status: "passed".to_owned(),
            reused: false,
        }],
        review_count: 1,
        known_unverified_environments: environments,
        commit_trailers: vec![
            "Slice-Review: fresh-context - completed - codex/test - clear".to_owned(),
        ],
    }
}

fn request(unverified: &str) -> Vec<u8> {
    format!(
        r#"{{
  "schema": "yo.slice-close-prepare-request/v1alpha1",
  "slice": "sample",
  "gate_request_path": "gate.json",
  "execution_lanes": [{{
    "lane": "integration",
    "mode": "serial",
    "operation_count": 1,
    "max_concurrency": 1
  }}],
  "review": {{
    "rounds": 1,
    "findings": {{
      "reported": 0,
      "resolved": 0,
      "not_reproduced": 0,
      "accepted_limits": 0,
      "remaining": 0
    }}
  }},
  "review_packets": {{
    "publication_count": 0,
    "total_managed_tokens": 0,
    "largest_sections": [],
    "reused_inputs": []
  }},
  "unverified_validation": {unverified},
  "elapsed_bottleneck": {{
    "name": "review",
    "elapsed_milliseconds": 1000
  }}
}}"#
    )
    .into_bytes()
}

// ready gate가 이미 검증한 validation command와 exact 후보를 그대로 투영하고,
// 관측 입력은 작업 레인·리뷰 회차·병목처럼 게이트가 알 수 없는 값만 보탠다.
#[test]
fn derives_identifiers_and_validation_from_ready_gate() {
    let bytes = build_metrics_for_test(
        &request("[]"),
        &gate(Vec::new()),
        "1111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222",
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(
        value["slice_candidate"],
        "1111111111111111111111111111111111111111"
    );
    assert_eq!(
        value["accepted_commit"],
        "2222222222222222222222222222222222222222"
    );
    assert_eq!(value["validation"][0]["name"], "xtask");
    assert_eq!(value["validation"][0]["argv"][0], "cargo");
    assert_eq!(value["validation"][0]["runs"], 1);
    assert_eq!(value["validation"][0]["status"], "passed");
}

// 미검증 환경은 이름만 복사해 누락시키지 않고, 실행하지 못한 command 하나와
// 일대일로 대응할 때만 표준 close metrics로 승격한다.
#[test]
fn requires_one_unverified_command_per_gate_environment() {
    let error = build_metrics_for_test(
        &request("[]"),
        &gate(vec!["macOS host unavailable".to_owned()]),
        "1111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222",
    )
    .unwrap_err();

    assert!(error.contains("map one-to-one"));
}
