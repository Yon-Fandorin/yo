use std::{fs, path};

use serde_json::{Value, json};

use super::{output::POLICY, run};
use crate::{git, review_protocol::digest, test_support::TestRepository};

struct Fixture {
    repository: TestRepository,
    request: path::PathBuf,
    output: path::PathBuf,
    source: path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let repository = TestRepository::new("cost-report");
        repository.write("base", "base\n");
        repository.git(["add", "base"]);
        repository.git(["commit", "-qm", "base"]);
        let source = repository.path.join("usage.json");
        fs::write(&source, br#"{"schema":"example.usage/v1","value":1}"#).unwrap();
        let request = repository.path.join("request.json");
        let output = repository.path.join("report.json");
        let head = git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        let source_ref = json!({
            "path": source,
            "hash": digest(&fs::read(&source).unwrap()),
            "schema": "example.usage/v1"
        });
        let reported = json!({"availability":"reported","value":10});
        let unavailable = json!({"availability":"unavailable","reason":"not exposed"});
        let document = json!({
            "schema": "yo.slice-cost-report-request/v1alpha1",
            "slice": "cost-report",
            "candidate_commit": head,
            "owners": {
                "packet": {"basis":"manifest metrics","sources":[],"publication_count":1,"rendered_bytes":reported,"managed_tokens":{"availability":"reported","value":3}},
                "provider": {"basis":"provider receipt","sources":[source_ref],"request_count":1,"usage":usage(reported.clone())},
                "coordinator_context": {"basis":"host observation","sources":[],"usage":usage(unavailable)},
                "command_output": {"basis":"bounded log","sources":[],"command_count":1,"complete_log_bytes":{"availability":"reported","value":100},"returned_bytes":{"availability":"reported","value":10}},
                "elapsed": {"basis":"wall clock","sources":[],"total_milliseconds":{"availability":"reported","value":1000},"critical_bottleneck":{"name":"tests","elapsed_milliseconds":900}}
            }
        });
        fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
        Self {
            repository,
            request,
            output,
            source,
        }
    }

    fn document(&self) -> Value {
        serde_json::from_slice(&fs::read(&self.request).unwrap()).unwrap()
    }

    fn write(&self, value: &Value) {
        fs::write(&self.request, serde_json::to_vec(value).unwrap()).unwrap();
    }
}

fn usage(value: Value) -> Value {
    json!({
        "input_tokens": value,
        "output_tokens": {"availability":"reported","value":2},
        "total_tokens": {"availability":"partial","value":12,"reason":"one round absent"},
        "reasoning_tokens": {"availability":"unavailable","reason":"not exposed"},
        "cache_read_input_tokens": {"availability":"reported","value":4},
        "cache_write_input_tokens": {"availability":"unavailable","reason":"not exposed"}
    })
}

// report는 다섯 owner를 그대로 보존하며 서로 다른 단위의 grand total을 만들지 않는다.
#[test]
fn publishes_owner_separated_report_without_cross_owner_total() {
    let fixture = Fixture::new();
    run(&fixture.repository.path, &fixture.request, &fixture.output).unwrap();
    let report: Value = serde_json::from_slice(&fs::read(&fixture.output).unwrap()).unwrap();
    assert_eq!(report["aggregation_policy"], POLICY);
    assert!(report["owners"]["packet"].is_object());
    assert!(report.get("total").is_none());
    assert_eq!(report["source_artifacts"][0]["bytes"], 39);
}

// source bytes나 schema가 request의 content address와 다르면 수치를 발행하지 않는다.
#[test]
fn rejects_stale_hash_or_schema() {
    let fixture = Fixture::new();
    fs::write(
        &fixture.source,
        br#"{"schema":"example.usage/v1","value":2}"#,
    )
    .unwrap();
    assert!(
        run(&fixture.repository.path, &fixture.request, &fixture.output)
            .unwrap_err()
            .contains("hash changed")
    );

    let fixture = Fixture::new();
    let mut document = fixture.document();
    document["owners"]["provider"]["sources"][0]["schema"] = json!("other/v1");
    fixture.write(&document);
    assert!(
        run(&fixture.repository.path, &fixture.request, &fixture.output)
            .unwrap_err()
            .contains("schema changed")
    );
}

// 하나의 artifact를 서로 다른 owner에 넣어 같은 비용을 두 번 세는 요청은 닫힌다.
#[test]
fn rejects_cross_owner_double_counting() {
    let fixture = Fixture::new();
    let mut document = fixture.document();
    document["owners"]["packet"]["sources"] = document["owners"]["provider"]["sources"].clone();
    fixture.write(&document);
    assert!(
        run(&fixture.repository.path, &fixture.request, &fixture.output)
            .unwrap_err()
            .contains("more than once")
    );
}

// command 반환량과 elapsed bottleneck은 자기 owner의 complete 관측을 넘을 수 없다.
#[test]
fn rejects_inverted_command_or_elapsed_measurements() {
    let fixture = Fixture::new();
    let mut document = fixture.document();
    document["owners"]["command_output"]["returned_bytes"]["value"] = json!(101);
    fixture.write(&document);
    assert!(
        run(&fixture.repository.path, &fixture.request, &fixture.output)
            .unwrap_err()
            .contains("complete log")
    );

    let fixture = Fixture::new();
    let mut document = fixture.document();
    document["owners"]["elapsed"]["total_milliseconds"]["value"] = json!(899);
    fixture.write(&document);
    assert!(
        run(&fixture.repository.path, &fixture.request, &fixture.output)
            .unwrap_err()
            .contains("bottleneck")
    );
}

// conflicting output은 덮어쓰지 않지만 exact retry는 같은 report로 수렴한다.
#[test]
fn publication_is_create_only_and_exact_retry_is_idempotent() {
    let fixture = Fixture::new();
    run(&fixture.repository.path, &fixture.request, &fixture.output).unwrap();
    run(&fixture.repository.path, &fixture.request, &fixture.output).unwrap();
    fs::write(&fixture.output, b"conflict\n").unwrap();
    assert!(run(&fixture.repository.path, &fixture.request, &fixture.output).is_err());
    assert_eq!(fs::read(&fixture.output).unwrap(), b"conflict\n");
}
