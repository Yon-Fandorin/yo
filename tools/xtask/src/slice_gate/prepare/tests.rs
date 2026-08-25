use std::{cell::Cell, path::PathBuf};

use serde_json::{Value, json};

use super::{prepare_with, set_final_revalidate_hook};
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
                "allowed_write_set": ["tools/**"],
                "focused_checks": ["cargo test --locked -p xtask slice_gate"],
                "slice_close_checks": ["cargo test --locked -p xtask"]
            }))
            .unwrap()
        );
        std::fs::write(&contract, &contract_bytes).unwrap();
        slice_contract::bind(&repository.path, &contract).unwrap();

        let validation = artifacts.join("validation.json");
        std::fs::write(
            &validation,
            br#"{"schema":"yo.validation-run-summary/v1","name":"xtask","status":"passed","exit_code":0,"elapsed_seconds":2,"log_bytes":42,"log_path":".local-exclude/validation-runs/xtask.log"}"#,
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

fn git_line(repository: &TestRepository, arguments: &[&str]) -> String {
    crate::git::output_in(&repository.path, arguments, false)
        .unwrap()
        .trim()
        .to_owned()
}

fn hash(byte: u8) -> String {
    format!("sha256:{}", format!("{byte:02x}").repeat(32))
}
