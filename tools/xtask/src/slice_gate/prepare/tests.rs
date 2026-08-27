use std::{cell::Cell, path::PathBuf};

use serde_json::{Value, json};

use super::{
    model::{
        CanonicalApprovalReviewCarryResult, REQUEST_SCHEMA_V1_ALPHA1, REQUEST_SCHEMA_V1_ALPHA2,
        RESULT_SCHEMA_V1_ALPHA1, RESULT_SCHEMA_V1_ALPHA2, REVIEW_CARRY_SCHEMA,
    },
    prepare_with, set_final_revalidate_hook,
};
use crate::{
    review_packet::{VerifiedEvidence, VerifiedReview},
    review_protocol::digest,
    slice_contract,
    test_support::{TestRepository, unique_path},
};

struct Fixture {
    repository: TestRepository,
    artifacts: PathBuf,
    prepare_path: PathBuf,
    output_path: PathBuf,
    request: Value,
    review: VerifiedReview,
}

impl Fixture {
    fn new() -> Self {
        let repository = TestRepository::new("slice-gate-prepare");
        repository.write("README", "base\n");
        repository.git(["add", "README"]);
        repository.git(["commit", "--quiet", "-m", "base"]);
        let base = git_line(&repository, &["rev-parse", "HEAD"]);
        repository.git(["switch", "--quiet", "-c", "slice/direct/gate-prepare-test"]);
        repository.write("tools/example.rs", "pub fn example() {}\n");
        repository.git(["add", "tools/example.rs"]);
        repository.git(["commit", "--quiet", "-m", "candidate"]);
        let candidate = git_line(&repository, &["rev-parse", "HEAD"]);

        let artifacts = unique_path("slice-gate-prepare-artifacts");
        std::fs::create_dir_all(&artifacts).unwrap();
        let contract = artifacts.join("slice-contract.json");
        let contract_bytes = format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "schema": "yo.slice-contract/v1",
                "slice": "gate-prepare-test",
                "base": base,
                "base_ref": "refs/heads/develop",
                "owned_contracts": ["repository.slice-gate.prepare.test"],
                "dependencies": [],
                "allowed_write_set": ["tools/**", "methexis/**"],
                "focused_checks": ["cargo test --locked -p xtask slice_gate"],
                "slice_close_checks": ["cargo test --locked -p xtask"]
            }))
            .unwrap()
        );
        std::fs::write(&contract, &contract_bytes).unwrap();
        slice_contract::bind(&repository.path, &contract).unwrap();

        let validation = artifacts.join("validation.json");
        let argv = ["cargo", "test", "--locked", "-p", "xtask"]
            .map(str::to_owned)
            .to_vec();
        std::fs::write(
            &validation,
            serde_json::to_vec(&json!({
                "schema": "yo.validation-run-summary/v1alpha2",
                "name": "xtask",
                "status": "passed",
                "exit_code": 0,
                "elapsed_seconds": 2,
                "log_bytes": 42,
                "log_path": ".local-exclude/validation-runs/xtask.log",
                "log_hash": digest(b"validation log"),
                "head_commit": candidate,
                "worktree_state": "clean",
                "command_argv_count": argv.len(),
                "command_argv_hash": crate::validation_summary::argv_hash(&argv),
                "reused": false,
                "reuse_policy": "reviewed-descendant/v1"
            }))
            .unwrap(),
        )
        .unwrap();
        let response = artifacts.join("review.txt");
        std::fs::write(&response, b"fresh-context clear; code-quality clear\n").unwrap();
        let manifest = artifacts.join("manifest.json");
        std::fs::write(&manifest, b"published manifest placeholder\n").unwrap();
        let review_id = hash(7);
        let packet_hash = hash(8);
        let receipt = artifacts.join("delivery.json");
        std::fs::write(
            &receipt,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({
                    "schema": "yo.external-review-delivery-receipt/v1",
                    "review_id": review_id,
                    "packet_hash": packet_hash,
                    "route": {
                        "provider": "qwencloud",
                        "account": "default",
                        "model": "qwen3.8-max"
                    },
                    "session_id": "review-session",
                    "provider_request_id": "request-1",
                    "provider_request_count": 1
                }))
                .unwrap()
            ),
        )
        .unwrap();

        let review = VerifiedReview {
            review_id,
            manifest_path: manifest.to_string_lossy().into_owned(),
            manifest_hash: digest(b"published manifest placeholder\n"),
            packet_path: "packet.md".to_owned(),
            packet_hash,
            base_commit: base,
            candidate_commit: candidate,
            trusted_commit: git_line(&repository, &["rev-parse", "develop"]),
            slice_contract_path: contract.to_string_lossy().into_owned(),
            slice_contract_hash: digest(contract_bytes.as_bytes()),
            validation_evidence: vec![VerifiedEvidence {
                name: "xtask".to_owned(),
                path: validation.to_string_lossy().into_owned(),
                hash: digest(&std::fs::read(&validation).unwrap()),
            }],
            review_lenses: vec!["fresh-context".to_owned(), "code-quality".to_owned()],
            review_questions: vec!["Does the gate preserve exact identity?".to_owned()],
        };
        let request = json!({
            "schema": "yo.slice-gate-prepare-request/v1",
            "manifest_path": manifest,
            "validation_commands": [{
                "name": "xtask",
                "argv": ["cargo", "test", "--locked", "-p", "xtask"],
                "reused": false
            }],
            "review_runs": [{
                "source": {
                    "kind": "delivery_receipt",
                    "receipt_path": receipt,
                    "class": "model-high"
                },
                "result_path": response,
                "verdicts": [
                    {"lens": "fresh-context", "verdict": "clear"},
                    {"lens": "code-quality", "verdict": "clear"}
                ]
            }],
            "known_unverified_environments": [],
            "risk": {
                "classification": "human-attention",
                "rationale": "changes workflow authority"
            },
            "approval": {
                "kind": "exact_candidate",
                "authority": "human/yon",
                "scope": "exact prepared candidate"
            }
        });
        let prepare_path = artifacts.join("prepare.json");
        std::fs::write(
            &prepare_path,
            format!("{}\n", serde_json::to_string_pretty(&request).unwrap()),
        )
        .unwrap();
        let output_path = artifacts.join("gate.json");
        Self {
            repository,
            artifacts,
            prepare_path,
            output_path,
            request,
            review,
        }
    }

    fn publish(&self) -> Result<super::model::PrepareResult, String> {
        self.publish_with_carry(None)
    }

    fn publish_with_carry(
        &self,
        carry: Option<CanonicalApprovalReviewCarryResult>,
    ) -> Result<super::model::PrepareResult, String> {
        let expected_path = PathBuf::from(&self.review.manifest_path);
        let expected_hash = self.review.manifest_hash.clone();
        let review = self.review.clone();
        let calls = Cell::new(0usize);
        let result = prepare_with(
            &self.repository.path,
            &self.prepare_path,
            &self.output_path,
            &|_, path, hash| {
                assert_eq!(path, expected_path);
                assert_eq!(hash, expected_hash);
                calls.set(calls.get() + 1);
                Ok(review.clone())
            },
            &|_, _, _, _| {
                carry
                    .clone()
                    .ok_or_else(|| "unexpected review carry in v1 fixture".to_owned())
            },
        );
        if result.is_ok() {
            assert_eq!(calls.get(), 2);
        }
        result
    }

    fn rewrite_request(&self) {
        std::fs::write(
            &self.prepare_path,
            format!("{}\n", serde_json::to_string_pretty(&self.request).unwrap()),
        )
        .unwrap();
    }

    fn use_external_operation(&mut self) {
        let argv = vec![
            "cargo".to_owned(),
            "test".to_owned(),
            "--locked".to_owned(),
            "-p".to_owned(),
            "xtask".to_owned(),
        ];
        let path = PathBuf::from(&self.review.validation_evidence[0].path);
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": "yo.external-operation-evidence/v1",
                "candidate_commit": self.review.candidate_commit,
                "operation": {
                    "working_directory": ".",
                    "argv": argv,
                    "expected_exit": {"kind": "code", "value": 0},
                    "observed_exit": {"kind": "code", "value": 0}
                },
                "counterfactual": "the operation must fail when the behavior regresses",
                "observations": [{
                    "name": "HEAD",
                    "expected_relation": "equal",
                    "before": self.review.candidate_commit,
                    "after": self.review.candidate_commit
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        self.review.validation_evidence[0].name = "external-operation/xtask".to_owned();
        self.review.validation_evidence[0].hash = digest(&std::fs::read(&path).unwrap());
        self.request["validation_commands"][0]["name"] = json!("external-operation/xtask");
        self.rewrite_request();
    }

    fn use_structured_result(&mut self) {
        self.request["schema"] = json!(REQUEST_SCHEMA_V1_ALPHA2);
        self.request["review_runs"][0]
            .as_object_mut()
            .unwrap()
            .remove("verdicts");
        let response = PathBuf::from(
            self.request["review_runs"][0]["result_path"]
                .as_str()
                .unwrap(),
        );
        let result = json!({
            "schema": "yo.slice-review-result/v1alpha1",
            "review_id": self.review.review_id,
            "candidate_commit": self.review.candidate_commit,
            "verdicts": [
                {"lens": "code-quality", "verdict": "clear"},
                {"lens": "fresh-context", "verdict": "clear"}
            ],
            "findings": []
        });
        std::fs::write(
            response,
            format!(
                "review complete\n<<<YO-SLICE-REVIEW-RESULT>>>\n{}\n<<<YO-SLICE-REVIEW-RESULT-END>>>\n",
                serde_json::to_string(&result).unwrap()
            ),
        )
        .unwrap();
        self.rewrite_request();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.artifacts);
    }
}

// 리뷰 manifest와 전달 영수증이 소유한 후보·증거·provider Session identity를 다시
// 입력하지 않아도 exact gate request를 만들고 같은 호출에서 통합 가능 상태까지 판정한다.
#[test]
fn prepares_and_evaluates_exact_gate_request_from_existing_artifacts() {
    let fixture = Fixture::new();
    let result = fixture.publish().unwrap();

    assert_eq!(result.status, "written");
    assert_eq!(result.gate.next_action, "integrate");
    assert_eq!(
        result.gate.candidate_commit,
        fixture.review.candidate_commit
    );
    assert_eq!(result.review_id, fixture.review.review_id);
    assert!(result.gate.review.iter().all(|entry| {
        entry.reviewer == "qwencloud/review-session"
            && entry.route == "model-high/qwencloud/qwen3.8-max/review-session"
    }));

    let generated: Value =
        serde_json::from_slice(&std::fs::read(&fixture.output_path).unwrap()).unwrap();
    assert_eq!(
        generated["approval"]["candidate_commit"],
        fixture.review.candidate_commit
    );
    assert_eq!(generated["approval"]["diff_hash"], result.gate.diff_hash);
    assert_eq!(
        generated["validation_evidence"][0]["result_hash"],
        fixture.review.validation_evidence[0].hash
    );
}

// alpha2 준비는 response의 terminal envelope에서 exact lens verdict를 파생하여
// coordinator가 clear/findings를 다시 입력하지 않아도 같은 gate evidence를 만듭니다.
#[test]
fn structured_result_drives_gate_without_declared_verdicts() {
    let mut fixture = Fixture::new();
    fixture.use_structured_result();

    let result = fixture.publish().unwrap();
    assert_eq!(result.schema, RESULT_SCHEMA_V1_ALPHA2);
    assert_eq!(result.gate.next_action, "integrate");
    assert!(
        result
            .gate
            .review
            .iter()
            .all(|entry| entry.verdict == "clear")
    );
}

// legacy schema는 수기 verdict를 계속 요구하고 alpha2는 이를 금지하여 어느 쪽도
// 누락을 clear로 추정하거나 두 authority를 조용히 혼합하지 않습니다.
#[test]
fn prepare_schema_keeps_declared_and_structured_verdict_authority_disjoint() {
    let mut legacy = Fixture::new();
    legacy.request["review_runs"][0]
        .as_object_mut()
        .unwrap()
        .remove("verdicts");
    legacy.rewrite_request();
    assert!(
        legacy
            .publish()
            .unwrap_err()
            .contains("requires declared verdicts")
    );

    let mut structured = Fixture::new();
    structured.request["schema"] = json!(REQUEST_SCHEMA_V1_ALPHA2);
    structured.rewrite_request();
    assert!(
        structured
            .publish()
            .unwrap_err()
            .contains("does not permit declared verdicts")
    );
}

// delegated receipt는 Provider/Account를 만들지 않고 host/session만으로 exact coverage와
// matching compact reviewer identity를 생성합니다.
#[test]
fn prepares_delegated_host_coverage_without_provider_coordinates() {
    let mut fixture = Fixture::new();
    let receipt = PathBuf::from(
        fixture.request["review_runs"][0]["source"]["receipt_path"]
            .as_str()
            .unwrap(),
    );
    std::fs::write(
        &receipt,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "schema": "yo.external-review-delegated-delivery-receipt/v1alpha1",
                "review_id": fixture.review.review_id,
                "packet_hash": fixture.review.packet_hash,
                "target": {"kind": "delegated_host", "host": "codex"},
                "execution_profile": "yo.delegated-review-execution/v1alpha1",
                "session_id": "review-session",
                "host_request_id": "host-request-1",
                "host_request_count": 1
            }))
            .unwrap()
        ),
    )
    .unwrap();
    fixture.request["review_runs"][0]["source"]["class"] = json!("delegated-high");
    fixture.rewrite_request();

    let result = fixture.publish().unwrap();
    assert!(result.gate.review.iter().all(|entry| {
        entry.reviewer == "codex/review-session"
            && entry.route == "delegated-high/codex/review-session"
    }));
}

// 준비 파일이 리뷰 manifest에 없거나 누락된 검증 이름을 쓰면 자동 생성 과정이 임의로
// 증거를 버리거나 추가하지 않고 정확한 집합 불일치로 중단한다.
#[test]
fn validation_command_names_must_match_reviewed_artifacts_exactly() {
    let mut fixture = Fixture::new();
    fixture.request["validation_commands"][0]["name"] = json!("other");
    fixture.rewrite_request();

    assert_eq!(
        fixture.publish().unwrap_err(),
        "validation_commands must name every and only reviewed validation artifact"
    );
}

// review-chain manifest의 external-operation artifact는 prepare 단계에서도 같은
// 후보와 내장 argv를 검증하여 direct gate 수기 전사 없이 gate request로 파생한다.
#[test]
fn prepares_external_operation_evidence_from_review_chain() {
    let mut fixture = Fixture::new();
    fixture.use_external_operation();

    let result = fixture.publish().unwrap();
    assert_eq!(result.gate.next_action, "integrate");
    assert_eq!(result.gate.validation[0].name, "external-operation/xtask");
    assert_eq!(result.gate.validation[0].status, "passed");
}

// 첫 capture 뒤 준비 파일이 바뀌면 새 approval나 route가 조용히 섞인 gate request를
// 발행하지 않고 publication 직전의 byte identity 검사에서 멈춘다.
#[test]
fn preparation_request_change_before_publication_fails_closed() {
    let fixture = Fixture::new();
    let path = fixture.prepare_path.clone();
    set_final_revalidate_hook(move || {
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.push(b' ');
        std::fs::write(&path, bytes).unwrap();
        Ok(())
    });

    assert_eq!(
        fixture.publish().unwrap_err(),
        "Slice gate preparation request changed before publication"
    );
    assert!(!fixture.output_path.exists());
}

// 안정 v1 요청은 새 필드를 조용히 받아들이지 않고, carry가 있는 실험 계약만
// v1alpha1로 명시하게 한다.
#[test]
fn stable_prepare_schema_rejects_review_carry() {
    let mut fixture = Fixture::new();
    fixture.request["review_carry"] = json!({
        "schema": REVIEW_CARRY_SCHEMA,
        "knowledge_id": "example.unit"
    });
    fixture.rewrite_request();

    assert_eq!(
        fixture.publish().unwrap_err(),
        "schema `yo.slice-gate-prepare-request/v1` does not permit review_carry"
    );
}

// 안정 v1은 새 필드의 null도 과거 unknown-field 실패처럼 거부한다.
#[test]
fn stable_prepare_schema_rejects_null_review_carry() {
    let mut fixture = Fixture::new();
    fixture.request["review_carry"] = Value::Null;
    fixture.rewrite_request();

    assert_eq!(
        fixture.publish().unwrap_err(),
        "schema `yo.slice-gate-prepare-request/v1` does not permit review_carry"
    );
}

// 실험 스키마는 carry 없이 기존 v1의 별칭으로 사용하지 못한다.
#[test]
fn alpha_prepare_schema_requires_review_carry() {
    let mut fixture = Fixture::new();
    fixture.request["schema"] = json!(REQUEST_SCHEMA_V1_ALPHA1);
    fixture.rewrite_request();

    assert_eq!(
        fixture.publish().unwrap_err(),
        "schema `yo.slice-gate-prepare-request/v1alpha1` requires review_carry"
    );
}

// 실험 스키마의 null은 carry 증거로 해석되지 않고 누락과 같이 닫혀 실패한다.
#[test]
fn alpha_prepare_schema_rejects_null_review_carry() {
    let mut fixture = Fixture::new();
    fixture.request["schema"] = json!(REQUEST_SCHEMA_V1_ALPHA1);
    fixture.request["review_carry"] = Value::Null;
    fixture.rewrite_request();

    assert_eq!(
        fixture.publish().unwrap_err(),
        "schema `yo.slice-gate-prepare-request/v1alpha1` requires review_carry"
    );
}

// 이미 검토된 semantic 후보의 strict descendant가 canonical approval 한 경로만
// 더할 때, current candidate identity로 gate evidence를 다시 파생하고 alpha2 검증
// 증거만 ancestor reuse로 받는다.
#[test]
fn prepares_exact_gate_for_canonical_approval_descendant() {
    let mut fixture = Fixture::new();
    fixture.repository.write(
        "methexis/approvals/example.unit.yaml",
        "canonical approval\n",
    );
    fixture
        .repository
        .git(["add", "methexis/approvals/example.unit.yaml"]);
    fixture
        .repository
        .git(["commit", "--quiet", "-m", "canonical approval"]);
    let candidate = git_line(&fixture.repository, &["rev-parse", "HEAD"]);

    fixture.request["schema"] = json!(REQUEST_SCHEMA_V1_ALPHA1);
    fixture.request["review_carry"] = json!({
        "schema": REVIEW_CARRY_SCHEMA,
        "knowledge_id": "example.unit"
    });
    fixture.request["validation_commands"][0]["reused"] = json!(true);
    fixture.rewrite_request();

    let carry = CanonicalApprovalReviewCarryResult {
        schema: REVIEW_CARRY_SCHEMA,
        knowledge_id: "example.unit".to_owned(),
        reviewed_candidate: fixture.review.candidate_commit.clone(),
        candidate_commit: candidate.clone(),
        knowledge_path: "methexis/knowledge/agent-runtime/example.unit.md".to_owned(),
        approval_path: "methexis/approvals/example.unit.yaml".to_owned(),
        revision: digest(b"knowledge"),
        reviewer: "human/owner".to_owned(),
        reviewed_at: "2026-08-26".to_owned(),
        request_hash: digest(b"request"),
        approval_hash: digest(b"canonical approval\n"),
        replaced_revision: None,
        transition_diff_hash: digest(b"transition"),
    };
    let result = fixture.publish_with_carry(Some(carry)).unwrap();

    assert_eq!(result.schema, RESULT_SCHEMA_V1_ALPHA1);
    assert_eq!(result.gate.candidate_commit, candidate);
    assert!(result.gate.validation[0].reused);
    let generated: Value =
        serde_json::from_slice(&std::fs::read(&fixture.output_path).unwrap()).unwrap();
    assert!(
        generated["review_evidence"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["candidate_commit"] == candidate)
    );
    assert_eq!(
        result.review_carry.unwrap().reviewed_candidate,
        fixture.review.candidate_commit
    );
}

fn git_line(repository: &TestRepository, arguments: &[&str]) -> String {
    crate::git::output_in(&repository.path, arguments, false)
        .unwrap()
        .trim()
        .to_owned()
}

fn hash(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}
