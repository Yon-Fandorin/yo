use std::path::{Path, PathBuf};

use serde_json::json;

use super::super::{check_readiness, readiness::set_test_hook};
use crate::{
    slice_contract,
    test_support::{TestRepository, unique_path},
};

// 실제 readiness 진입점은 ContextBuild나 review artifact를 만들지 않고 성공 결과를 내며,
// capture 뒤 validation이 바뀌면 결과 바이트 없이 final guard에서 실패한다.
#[test]
fn readiness_production_boundary_is_non_publishing_and_fails_closed() {
    let fixture = ReadinessFixture::new();
    let mut ready_output = Vec::new();

    check_readiness(
        &fixture.repository.path,
        &fixture.request_path,
        &mut ready_output,
    )
    .unwrap();

    let ready: serde_json::Value = serde_json::from_slice(&ready_output).unwrap();
    assert_eq!(ready["status"], "input_boundaries_ready");
    assert_eq!(ready["artifacts_published"], false);
    assert!(ready.get("review_id").is_none());
    assert!(!fixture.artifact_root.exists());

    let validation = fixture.validation_path.clone();
    set_test_hook(move || {
        std::fs::write(validation, b"changed after capture\n")
            .map_err(|error| format!("cannot mutate validation fixture: {error}"))
    });
    let mut changed_output = Vec::new();
    let error = check_readiness(
        &fixture.repository.path,
        &fixture.request_path,
        &mut changed_output,
    )
    .unwrap_err();

    assert_eq!(error, "validation evidence changed during readiness check");
    assert!(changed_output.is_empty());
    assert!(!fixture.artifact_root.exists());
}

// capture 뒤 같은 commit을 가리키는 허용 가능한 Task branch로 바뀌어도 최종 guard는
// 정확한 Slice branch 재검증에서 실패하여 준비 완료 결과를 내지 않는다.
#[test]
fn readiness_final_guard_rejects_a_same_commit_task_branch_switch() {
    let fixture = ReadinessFixture::new();
    let repository = fixture.repository.path.clone();
    set_test_hook(move || {
        crate::git::output_in(
            &repository,
            &[
                "switch",
                "--quiet",
                "-c",
                "task/direct/readiness-fixture/late-drift",
            ],
            false,
        )
        .map(|_| ())
    });
    let mut output = Vec::new();

    let error =
        check_readiness(&fixture.repository.path, &fixture.request_path, &mut output).unwrap_err();

    assert_eq!(
        error,
        "trusted Git branch does not match bound Slice; expected \
         refs/heads/slice/direct/readiness-fixture"
    );
    assert!(output.is_empty());
    assert!(!fixture.artifact_root.exists());
}

// candidate worktree 밖의 ContextBuild request는 Methexis를 호출하기 전에 거부되어,
// 외부 경로 오류가 context artifact 생성 뒤에야 드러나는 회귀를 막는다.
#[test]
fn readiness_rejects_an_external_context_request_before_context_build() {
    let fixture = ReadinessFixture::new();
    let external = unique_path("external-context-request");
    std::fs::write(&external, b"{}\n").unwrap();
    fixture.write_review_request(&external, &["validation"]);
    let mut output = Vec::new();

    let error =
        check_readiness(&fixture.repository.path, &fixture.request_path, &mut output).unwrap_err();

    assert_eq!(
        error,
        "Methexis ContextBuild request must be inside the candidate worktree"
    );
    assert!(output.is_empty());
    assert!(!fixture.artifact_root.exists());
    std::fs::remove_file(external).unwrap();
}

// 서로 다른 이름으로 같은 validation 파일을 두 번 싣는 request는 section 의미를
// 중복시키므로 readiness와 publication이 공유하는 capture 경계에서 거부한다.
#[test]
fn readiness_rejects_duplicate_validation_paths() {
    let fixture = ReadinessFixture::new();
    fixture.write_review_request(&fixture.context_path, &["validation-a", "validation-b"]);
    let mut output = Vec::new();

    let error =
        check_readiness(&fixture.repository.path, &fixture.request_path, &mut output).unwrap_err();

    assert_eq!(error, "validation evidence paths must be unique");
    assert!(output.is_empty());
    assert!(!fixture.artifact_root.exists());
}

// 실제 argv와 exit, HEAD 전후 관계를 기록한 구조화 evidence가 정확한 candidate를
// 가리키면 readiness가 별도 실행 없이 그 경계를 고정하고 개수를 보고한다.
#[test]
fn readiness_accepts_candidate_bound_external_operation_evidence() {
    let fixture = ReadinessFixture::new();
    let evidence_path = fixture.repository.write(
        ".local-exclude/readiness/git-amend.json",
        &external_evidence(&fixture.candidate_commit, 1, 1, "equal", "head", "head"),
    );
    fixture.write_review_request_entries(vec![json!({
        "name": "external-operation/git-amend-file",
        "path": relative(&fixture.repository.path, &evidence_path)
    })]);
    let mut output = Vec::new();

    check_readiness(&fixture.repository.path, &fixture.request_path, &mut output).unwrap();

    let result: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(result["external_operation_evidence_count"], 1);
    assert_eq!(result["validation_evidence_count"], 1);
    assert!(!fixture.artifact_root.exists());
}

// 이전 candidate의 외부 동작 결과는 내용과 관계가 그럴듯해도 현재 patch의 증거가
// 아니므로 readiness가 ContextBuild와 review artifact 생성 전에 거부한다.
#[test]
fn readiness_rejects_external_operation_evidence_for_another_candidate() {
    let fixture = ReadinessFixture::new();
    let evidence_path = fixture.repository.write(
        ".local-exclude/readiness/stale-operation.json",
        &external_evidence(
            "0000000000000000000000000000000000000000",
            1,
            1,
            "equal",
            "head",
            "head",
        ),
    );
    fixture.write_review_request_entries(vec![json!({
        "name": "external-operation/stale-amend",
        "path": relative(&fixture.repository.path, &evidence_path)
    })]);
    let mut output = Vec::new();

    let error =
        check_readiness(&fixture.repository.path, &fixture.request_path, &mut output).unwrap_err();

    assert!(error.contains("does not identify the exact candidate commit"));
    assert!(output.is_empty());
    assert!(!fixture.artifact_root.exists());
}

// expected/observed exit가 다르거나 equal 관찰의 실제 전후 값이 다르면 단순한
// 성공 선언을 discriminating counterfactual로 포장하지 못하게 각각 거부한다.
#[test]
fn external_operation_schema_rejects_logically_inconsistent_evidence() {
    for (label, evidence, expected) in [
        (
            "exit",
            external_evidence(
                "1111111111111111111111111111111111111111",
                1,
                0,
                "equal",
                "head",
                "head",
            ),
            "different exit status",
        ),
        (
            "relation",
            external_evidence(
                "1111111111111111111111111111111111111111",
                1,
                1,
                "equal",
                "before",
                "after",
            ),
            "contradicts its expected relation",
        ),
    ] {
        let candidate = "1111111111111111111111111111111111111111";
        let error = super::super::external_operation::validate(
            &format!("external-operation/{label}"),
            evidence.as_bytes(),
            candidate,
        )
        .unwrap_err();
        assert!(error.contains(expected), "{label}: {error}");
    }
}

struct ReadinessFixture {
    repository: TestRepository,
    contract_path: PathBuf,
    request_path: PathBuf,
    context_path: PathBuf,
    validation_path: PathBuf,
    artifact_root: PathBuf,
    candidate_commit: String,
}

impl ReadinessFixture {
    fn new() -> Self {
        let repository = TestRepository::new("review-request-readiness");
        repository.write(".gitignore", ".local-exclude/\n");
        repository.write("CONTRIBUTING.md", "review authority\n");
        repository.git(["add", ".gitignore", "CONTRIBUTING.md"]);
        repository.git(["commit", "--quiet", "-m", "readiness fixture base"]);
        let base = crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
            .unwrap()
            .trim()
            .to_owned();
        repository.git(["switch", "--quiet", "-c", "slice/direct/readiness-fixture"]);
        repository.write("candidate.txt", "candidate\n");
        repository.git(["add", "candidate.txt"]);
        repository.git(["commit", "--quiet", "-m", "readiness fixture candidate"]);
        let candidate_commit =
            crate::git::output_in(&repository.path, &["rev-parse", "HEAD"], false)
                .unwrap()
                .trim()
                .to_owned();

        let contract_path = repository.write(
            ".git/readiness-contract.json",
            &json_text(&json!({
                "schema": "yo.slice-contract/v1",
                "slice": "readiness-fixture",
                "base": base,
                "base_ref": "refs/heads/develop",
                "owned_contracts": ["repository.review-request.readiness"],
                "dependencies": ["repository.review-packet.preflight"],
                "allowed_write_set": ["candidate.txt"],
                "focused_checks": ["cargo test --locked -p xtask review_packet"],
                "slice_close_checks": ["cargo test --locked -p xtask"]
            })),
        );
        slice_contract::bind(&repository.path, &contract_path).unwrap();
        let context_path = repository.write(
            ".local-exclude/readiness/context-request.json",
            &json_text(&json!({
                "schema": "methexis.context-request/v1alpha1",
                "anchors": [{
                    "kind": "knowledge_id",
                    "value": "methexis.review.bounded-packet"
                }],
                "tokenizer_profile": "o200k_base/v1",
                "max_tokens": 16000
            })),
        );
        let validation_path = repository.write(
            ".local-exclude/readiness/validation.md",
            "validation passed\n",
        );
        let request_path = repository
            .path
            .join(".local-exclude/readiness/review-request.json");
        let artifact_root = repository.path.join(".local-exclude/methexis");
        let fixture = Self {
            repository,
            contract_path,
            request_path,
            context_path,
            validation_path,
            artifact_root,
            candidate_commit,
        };
        fixture.write_review_request(&fixture.context_path, &["validation"]);
        assert!(
            crate::git::output_in(&fixture.repository.path, &["status", "--porcelain"], false)
                .unwrap()
                .is_empty()
        );
        fixture
    }

    fn write_review_request(&self, context_path: &Path, validation_names: &[&str]) {
        let validation = validation_names
            .iter()
            .map(|name| {
                json!({
                    "name": name,
                    "path": relative(&self.repository.path, &self.validation_path)
                })
            })
            .collect::<Vec<_>>();
        self.write_review_request_with_context(context_path, validation);
    }

    fn write_review_request_entries(&self, validation: Vec<serde_json::Value>) {
        self.write_review_request_with_context(&self.context_path, validation);
    }

    fn write_review_request_with_context(
        &self,
        context_path: &Path,
        validation: Vec<serde_json::Value>,
    ) {
        self.repository.write(
            ".local-exclude/readiness/review-request.json",
            &json_text(&json!({
                "schema": "yo.slice-review-packet-request/v1",
                "context_request_path": path_value(&self.repository.path, context_path),
                "required_knowledge_ids": ["methexis.review.bounded-packet"],
                "slice_contract_path": self.contract_path,
                "repository_authority_paths": ["CONTRIBUTING.md"],
                "validation_evidence": validation,
                "review_lenses": ["fresh-context", "code-quality"],
                "review_questions": ["Is the readiness boundary correct?"],
                "delivery_profile": "yo.slice-review-markdown/v1",
                "tokenizer_profile": "o200k_base/v1",
                "max_managed_payload_tokens": 90000
            })),
        );
    }
}

fn external_evidence(
    candidate: &str,
    expected_exit: i32,
    observed_exit: i32,
    relation: &str,
    before: &str,
    after: &str,
) -> String {
    json_text(&json!({
        "schema": "yo.external-operation-evidence/v1",
        "candidate_commit": candidate,
        "operation": {
            "working_directory": ".",
            "argv": ["git", "commit", "--amend", "--file", "message"],
            "expected_exit": {"kind": "code", "value": expected_exit},
            "observed_exit": {"kind": "code", "value": observed_exit}
        },
        "counterfactual": "The amend must fail before changing HEAD.",
        "observations": [{
            "name": "HEAD",
            "expected_relation": relation,
            "before": before,
            "after": after
        }]
    }))
}

fn json_text(value: &serde_json::Value) -> String {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    String::from_utf8(bytes).unwrap()
}

fn path_value(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).map_or_else(
        |_| path.to_string_lossy().into_owned(),
        |path| path.to_string_lossy().replace('\\', "/"),
    )
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}
