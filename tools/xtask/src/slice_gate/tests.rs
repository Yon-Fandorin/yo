use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{evaluate, set_final_revalidate_test_hook};
use crate::{review_protocol, slice_contract, test_support};

struct Fixture {
    repository: test_support::TestRepository,
    artifacts: PathBuf,
    request: Value,
    candidate: String,
    diff_hash: String,
}

impl Fixture {
    fn new() -> Self {
        let repository = test_support::TestRepository::new("slice-gate");
        repository.write("README", "base\n");
        repository.git(["add", "README"]);
        repository.git(["commit", "--quiet", "-m", "base"]);
        let base = git_line(&repository.path, &["rev-parse", "HEAD"]);
        repository.git(["switch", "--quiet", "-c", "slice/direct/gate-test"]);
        repository.write("tools/example.rs", "pub fn example() {}\n");
        repository.git(["add", "tools/example.rs"]);
        repository.git(["commit", "--quiet", "-m", "candidate"]);
        let candidate = git_line(&repository.path, &["rev-parse", "HEAD"]);
        let diff = crate::git::trusted_output_bytes_in(
            &repository.path,
            &[
                "diff",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-renames",
                &base,
                &candidate,
                "--",
            ],
        )
        .unwrap();
        let diff_hash = review_protocol::digest(&diff);

        let artifacts = test_support::unique_path("slice-gate-artifacts");
        std::fs::create_dir_all(&artifacts).unwrap();
        let contract = artifacts.join("slice-contract.json");
        std::fs::write(
            &contract,
            serde_json::to_vec_pretty(&json!({
                "schema": "yo.slice-contract/v1",
                "slice": "gate-test",
                "base": base,
                "base_ref": "refs/heads/develop",
                "owned_contracts": ["repository.workflow.single-slice-gate.test"],
                "dependencies": [],
                "allowed_write_set": ["tools/**"],
                "focused_checks": ["cargo test --locked -p xtask slice_gate"],
                "slice_close_checks": ["cargo test --workspace --all-targets"]
            }))
            .unwrap(),
        )
        .unwrap();
        slice_contract::bind(&repository.path, &contract).unwrap();

        let validation = artifacts.join("validation.json");
        std::fs::write(
            &validation,
            br#"{"schema":"yo.validation-run-summary/v1","name":"xtask","status":"passed","exit_code":0,"elapsed_seconds":2,"log_bytes":42,"log_path":".local-exclude/validation-runs/xtask.log"}"#,
        )
        .unwrap();
        let validation_hash = digest_file(&validation);
        let fresh = artifacts.join("fresh.txt");
        std::fs::write(&fresh, b"fresh review clear\n").unwrap();
        let fresh_hash = digest_file(&fresh);
        let quality = artifacts.join("quality.txt");
        std::fs::write(&quality, b"quality review clear\n").unwrap();
        let quality_hash = digest_file(&quality);

        let request = json!({
            "schema": "yo.slice-gate-request/v1alpha1",
            "candidate_commit": candidate,
            "required_lenses": ["fresh-context", "code-quality"],
            "validation_evidence": [{
                "name": "xtask",
                "argv": ["cargo", "test", "--locked", "-p", "xtask"],
                "result_path": validation,
                "result_hash": validation_hash,
                "candidate_commit": candidate,
                "reused": false
            }],
            "review_evidence": [{
                "lens": "fresh-context",
                "reviewer": "codex/fresh-session",
                "route": "model-high/codex/gpt-5.6-sol/fresh-session",
                "verdict": "clear",
                "candidate_commit": candidate,
                "diff_hash": diff_hash,
                "result_path": fresh,
                "result_hash": fresh_hash
            }, {
                "lens": "code-quality",
                "reviewer": "codex/quality-session",
                "route": "model/codex/gpt-5.6-luna/quality-session",
                "verdict": "clear",
                "candidate_commit": candidate,
                "diff_hash": diff_hash,
                "result_path": quality,
                "result_hash": quality_hash
            }],
            "known_unverified_environments": [],
            "risk": {
                "classification": "human-attention",
                "rationale": "changes workflow authority"
            },
            "approval": {
                "kind": "exact_candidate",
                "authority": "human/yon",
                "scope": "exact gate candidate",
                "candidate_commit": candidate,
                "diff_hash": diff_hash
            }
        });
        Self {
            repository,
            artifacts,
            request,
            candidate,
            diff_hash,
        }
    }

    fn evaluate(&self) -> Result<super::model::ResultDocument, String> {
        let request_path = self.artifacts.join("request.json");
        std::fs::write(&request_path, serde_json::to_vec(&self.request).unwrap()).unwrap();
        evaluate(&self.repository.path, &request_path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.artifacts);
    }
}

// 모든 증거와 정확 승인이 같은 후보와 diff에 결속되면 실행을 반복하지 않고
// 통합이 유일한 다음 행동임을 보고하고, 커밋에 넣을 exact trailer도 함께 만든다.
#[test]
fn ready_candidate_reports_integrate_and_exact_trailers() {
    let fixture = Fixture::new();
    let result = fixture.evaluate().unwrap();

    assert_eq!(result.next_action, "integrate");
    assert_eq!(result.status, "ready");
    assert_eq!(result.candidate_commit, fixture.candidate);
    assert_eq!(result.diff_hash, fixture.diff_hash);
    assert_eq!(result.commit_trailers.len(), 4);
    assert!(
        result
            .commit_trailers
            .iter()
            .all(|trailer| !trailer.starts_with("Review-Coverage:")
                || trailer.ends_with(&fixture.diff_hash))
    );
}

// 검증 증거가 아직 없으면 리뷰나 승인을 다시 요구하지 않고 검증만 다음 행동으로
// 선택하여 coordinator가 가장 이른 미충족 게이트부터 진행할 수 있게 한다.
#[test]
fn missing_validation_reports_validate() {
    let mut fixture = Fixture::new();
    fixture.request["validation_evidence"] = json!([]);

    assert_eq!(fixture.evaluate().unwrap().next_action, "validate");
}

// 실행된 command가 nonzero로 끝난 bounded summary는 증거가 존재하더라도 green으로
// 바꾸지 않으며, 실패 원인을 고친 뒤 검증할 차례임을 그대로 유지한다.
#[test]
fn failed_validation_reports_validate() {
    let mut fixture = Fixture::new();
    let path = PathBuf::from(
        fixture.request["validation_evidence"][0]["result_path"]
            .as_str()
            .unwrap(),
    );
    std::fs::write(
        &path,
        br#"{"schema":"yo.validation-run-summary/v1","name":"xtask","status":"failed","exit_code":1,"elapsed_seconds":2,"log_bytes":42,"log_path":".local-exclude/validation-runs/xtask.log"}"#,
    )
    .unwrap();
    fixture.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));

    assert_eq!(fixture.evaluate().unwrap().next_action, "validate");
}

// 필요한 렌즈 중 하나가 빠진 정확 후보는 검증이 통과했어도 준비 완료가 아니며,
// 기존에 끝난 렌즈를 반복하지 않고 리뷰만 다음 행동으로 표시한다.
#[test]
fn missing_review_reports_review() {
    let mut fixture = Fixture::new();
    fixture.request["review_evidence"]
        .as_array_mut()
        .unwrap()
        .pop();

    assert_eq!(fixture.evaluate().unwrap().next_action, "review");
}

// 검증과 리뷰가 완전해도 승인 객체가 없으면 human-attention 후보를 통합 가능으로
// 오인하지 않고 승인 하나만 남았음을 표시한다.
#[test]
fn missing_approval_reports_approve() {
    let mut fixture = Fixture::new();
    fixture.request["approval"] = Value::Null;

    assert_eq!(fixture.evaluate().unwrap().next_action, "approve");
}

// human approval은 정확히 두 segment인 human/<identity>만 받아 빈 identity나
// 추가 path segment가 standing/exact authorization으로 오인되지 않게 한다.
#[test]
fn malformed_human_approval_authority_fails_closed() {
    for authority in ["human/", "human/a/b"] {
        let mut fixture = Fixture::new();
        fixture.request["approval"]["authority"] = json!(authority);

        assert!(
            fixture
                .evaluate()
                .unwrap_err()
                .contains("exactly human/<identity>")
        );
    }
}

// request의 최상위 후보가 현재 clean HEAD와 다르면 하위 증거를 읽기 전에 거부하여
// 이전 후보의 녹색 결과가 새 후보로 승계되지 않게 한다.
#[test]
fn stale_candidate_fails_closed() {
    let mut fixture = Fixture::new();
    fixture.request["candidate_commit"] = json!("0000000000000000000000000000000000000000");

    assert!(fixture.evaluate().unwrap_err().contains("request is stale"));
}

// 리뷰가 다른 canonical diff를 가리키면 같은 commit 표기가 우연히 남아 있더라도
// exact review coverage로 인정하지 않는다.
#[test]
fn stale_review_diff_fails_closed() {
    let mut fixture = Fixture::new();
    fixture.request["review_evidence"][0]["diff_hash"] =
        json!("sha256:0000000000000000000000000000000000000000000000000000000000000000");

    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("review `fresh-context` is stale")
    );
}

// request 작성 뒤 evidence bytes가 바뀌면 경로 이름과 verdict가 그대로여도 hash
// 결속이 끊긴 것이므로 stale 결과를 사용하지 않고 실패한다.
#[test]
fn changed_evidence_bytes_fail_closed() {
    let fixture = Fixture::new();
    let path = PathBuf::from(
        fixture.request["review_evidence"][0]["result_path"]
            .as_str()
            .unwrap(),
    );
    std::fs::write(path, b"changed after request\n").unwrap();

    assert!(fixture.evaluate().unwrap_err().contains("hash changed"));
}

// 최초 capture 뒤 반환 직전에 evidence가 바뀌면 final revalidation이 같은 hash를
// 다시 읽어, 이미 만든 in-memory green 결과를 ready로 반환하지 않는다.
#[test]
fn final_revalidation_rejects_evidence_changed_after_capture() {
    let fixture = Fixture::new();
    let path = PathBuf::from(
        fixture.request["review_evidence"][0]["result_path"]
            .as_str()
            .unwrap(),
    );
    set_final_revalidate_test_hook(move || {
        std::fs::write(path, b"changed before final revalidation\n")
            .map_err(|error| error.to_string())
    });

    assert!(fixture.evaluate().unwrap_err().contains("hash changed"));
}

// 최초 identity 확인 뒤 request 자체가 바뀌면 마지막 regular-file capture가 이를
// 감지하여 다른 승인/증거 선언으로 바뀐 bytes에 이전 평가를 적용하지 않는다.
#[test]
fn final_revalidation_rejects_request_changed_after_capture() {
    let fixture = Fixture::new();
    let request_path = fixture.artifacts.join("request.json");
    set_final_revalidate_test_hook(move || {
        std::fs::write(request_path, b"{}\n").map_err(|error| error.to_string())
    });

    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("request changed during evaluation")
    );
}

// tools Rust 변경이 요구하는 path-derived 최소 렌즈를 planner가 request에서 빼도
// 게이트가 기존 impact 규칙을 재사용해 누락을 즉시 거부한다.
#[test]
fn request_cannot_omit_path_derived_lens() {
    let mut fixture = Fixture::new();
    fixture.request["required_lenses"] = json!(["fresh-context"]);

    assert!(fixture.evaluate().unwrap_err().contains("code-quality"));
}

// 이미 허용된 기계적 후속 작업은 명시된 human-origin standing authorization으로
// 준비 완료가 될 수 있어 exact merge 승인 문구를 불필요하게 다시 만들지 않는다.
#[test]
fn routine_candidate_accepts_standing_authorization() {
    let mut fixture = Fixture::new();
    fixture.request["risk"] = json!({
        "classification": "routine",
        "rationale": "mechanical follow-through under an accepted contract"
    });
    fixture.request["approval"] = json!({
        "kind": "standing_routine",
        "authority": "human/yon",
        "scope": "routine exact-contract implementation",
        "candidate_commit": null,
        "diff_hash": null
    });

    assert_eq!(fixture.evaluate().unwrap().next_action, "integrate");
}

// 실행하지 못한 필수 환경이 남은 후보는 routine 자동 경로로 흘리지 않고
// human-attention으로 분류해 사람이 그 공백을 명시적으로 판단하게 한다.
#[test]
fn routine_candidate_rejects_unverified_environment() {
    let mut fixture = Fixture::new();
    fixture.request["risk"] = json!({
        "classification": "routine",
        "rationale": "mechanical follow-through"
    });
    fixture.request["known_unverified_environments"] = json!(["registered macOS runner"]);

    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("routine risk cannot retain known unverified")
    );
}

fn digest_file(path: &Path) -> String {
    review_protocol::digest(&std::fs::read(path).unwrap())
}

fn git_line(repository: &Path, arguments: &[&str]) -> String {
    crate::git::trusted_output_in(repository, arguments)
        .unwrap()
        .trim()
        .to_owned()
}
