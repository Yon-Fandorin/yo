use std::{ffi::OsString, path::PathBuf, process::ExitCode};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate is nested below the repository root")
        .to_owned()
}

fn run_example(name: &str) -> (ExitCode, Vec<u8>, Vec<u8>) {
    let root = repository_root();
    let request = root
        .join("tools/librarian/examples/discovery-contract")
        .join(name);
    let arguments = vec![
        OsString::from("discover"),
        OsString::from("--repository"),
        root.into_os_string(),
        request.into_os_string(),
    ];
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = librarian::run(arguments, &mut stdout, &mut stderr).expect("CLI completes");
    (code, stdout, stderr)
}

fn success(name: &str) -> (serde_json::Value, Vec<u8>) {
    let (code, stdout, stderr) = run_example(name);
    assert_eq!(code, ExitCode::SUCCESS);
    assert!(stderr.is_empty());
    let value = serde_json::from_slice(&stdout).expect("candidate-set JSON");
    (value, stdout)
}

// 영어 query 결과에서 각 candidate 점수가 reason 점수의 합계로 설명되고 approval·eligibility 필드가
// 없음을 검증한다.
#[test]
fn canonical_english_returns_explained_candidates() {
    let (result, _) = success("query-english.json");

    assert_eq!(
        result["candidates"][0]["id"],
        "tui.architecture.module-boundaries"
    );
    assert!(
        result["candidates"][0]["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason["kind"] == "query_phrase")
    );
    assert!(
        result["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .skip(1)
            .flat_map(|candidate| candidate["reasons"].as_array().expect("reasons"))
            .any(|reason| reason["kind"] == "relation")
    );
    assert!(result.get("approval").is_none());
    assert!(result.get("eligibility").is_none());
    for candidate in result["candidates"].as_array().expect("candidates") {
        let explained = candidate["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .map(|reason| reason["score"].as_u64().expect("reason score"))
            .sum::<u64>();
        assert_eq!(candidate["score"], explained);
    }
}

// 한국어 query가 projection 필드 매칭으로 지식 단위를 검색할 수 있는지 확인한다.
#[test]
fn exact_revision_korean_projection_is_searchable() {
    let (result, _) = success("query-korean.json");

    assert_eq!(
        result["candidates"][0]["id"],
        "tui.dependencies.selection-gate"
    );
    assert!(
        result["candidates"][0]["reasons"]
            .as_array()
            .expect("reasons")
            .iter()
            .any(|reason| reason["field"] == "projection")
    );
}

// 코드 경로 앵커가 정확히 일치한 지식 단위를 관계 이웃보다 우선하는지 확인한다.
#[test]
fn applies_to_path_anchor_is_an_exact_signal() {
    let (result, _) = success("anchor-path.json");

    assert_eq!(
        result["candidates"][0]["id"],
        "tui.dependencies.selection-gate"
    );
    assert_eq!(result["candidates"][0]["score"], 8_000);
    assert_eq!(result["candidates"][0]["reasons"][0]["kind"], "anchor");
    assert!(
        result["candidates"]
            .as_array()
            .expect("candidates")
            .iter()
            .skip(1)
            .all(|candidate| candidate["score"].as_u64().expect("score") < 8_000)
    );
}

// knowledge_id 앵커가 정확히 일치하는 지식 단위를 최고 점수로 관계 이웃보다 우선시키는지 확인한다.
#[test]
fn exact_knowledge_id_anchor_outranks_relation_neighbors() {
    let (result, _) = success("anchor-id.json");

    assert_eq!(result["candidates"][0]["id"], "tui.runtime.typed-flow");
    assert_eq!(result["candidates"][0]["score"], 10_000);
    assert_eq!(
        result["candidates"][0]["reasons"][0]["anchor_kind"],
        "knowledge_id"
    );
}

// 매칭 결과가 없는 query도 빈 candidates와 빈 unresolved_anchors를 보고하며 성공하는지 확인한다.
#[test]
fn query_with_no_matches_is_a_successful_empty_observation() {
    let (result, _) = success("query-no-match.json");

    assert!(
        result["candidates"]
            .as_array()
            .expect("candidates")
            .is_empty()
    );
    assert!(
        result["unresolved_anchors"]
            .as_array()
            .expect("unresolved anchors")
            .is_empty()
    );
}

// 해석할 수 없는 앵커가 있어도 빈 candidates와 하나의 unresolved_anchors를 보고하며 성공하는지
// 확인한다.
#[test]
fn unresolved_anchor_is_a_successful_empty_observation() {
    let (result, _) = success("unresolved-anchor.json");

    assert!(
        result["candidates"]
            .as_array()
            .expect("candidates")
            .is_empty()
    );
    assert_eq!(
        result["unresolved_anchors"]
            .as_array()
            .expect("unresolved anchors")
            .len(),
        1
    );
}

// 동일한 snapshot과 요청에 대한 discover 출력이 바이트 단위로 결정적인지 확인한다.
#[test]
fn identical_snapshot_and_request_are_byte_deterministic() {
    let (_, first) = success("query-english.json");
    let (_, second) = success("query-english.json");

    assert_eq!(first, second);
}

// 성공 wire 출력 전체가 golden fixture와 정확히 일치하는지 검증한다.
#[test]
fn complete_success_wire_output_matches_the_golden_fixture() {
    let (_, stdout) = success("query-english.json");
    let expected = std::fs::read(
        repository_root()
            .join("tools/librarian/examples/discovery-contract")
            .join("expected-query-english.json"),
    )
    .expect("success fixture");

    assert_eq!(stdout, expected);
}

// 실패 시 종료 코드 2, 빈 stdout, 그리고 golden fixture와 일치하는 stderr를 검증한다.
#[test]
fn complete_failure_wire_output_matches_the_golden_fixture() {
    let (code, stdout, stderr) = run_example("failure-duplicate-anchor.json");
    let expected = std::fs::read(
        repository_root()
            .join("tools/librarian/examples/discovery-contract")
            .join("expected-failure-duplicate-anchor.json"),
    )
    .expect("failure fixture");

    assert_eq!(code, ExitCode::from(2));
    assert!(stdout.is_empty());
    assert_eq!(stderr, expected);
}
