use std::env;

use super::*;
use crate::validation_summary;

impl Fixture {
    fn use_alpha_validation(&mut self) {
        let argv = vec![
            "cargo".to_owned(),
            "test".to_owned(),
            "--locked".to_owned(),
            "-p".to_owned(),
            "xtask".to_owned(),
        ];
        let path = PathBuf::from(
            self.request["validation_evidence"][0]["result_path"]
                .as_str()
                .unwrap(),
        );
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": "yo.validation-run-summary/v1alpha1",
                "name": "xtask",
                "status": "passed",
                "exit_code": 0,
                "elapsed_seconds": 2,
                "log_bytes": 42,
                "log_path": ".local-exclude/validation-runs/xtask.log",
                "log_hash": format!("sha256:{}", "a".repeat(64)),
                "head_commit": self.candidate,
                "worktree_state": "clean",
                "command_argv_count": argv.len(),
                "command_argv_hash": validation_summary::argv_hash(&argv),
                "reused": false
            }))
            .unwrap(),
        )
        .unwrap();
        self.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));
    }

    fn use_external_operation(&mut self) {
        let argv = vec![
            "cargo".to_owned(),
            "test".to_owned(),
            "--locked".to_owned(),
            "-p".to_owned(),
            "xtask".to_owned(),
        ];
        let path = PathBuf::from(
            self.request["validation_evidence"][0]["result_path"]
                .as_str()
                .unwrap(),
        );
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": "yo.external-operation-evidence/v1",
                "candidate_commit": self.candidate,
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
                    "before": self.candidate,
                    "after": self.candidate
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        self.request["validation_evidence"][0]["name"] = json!("external-operation/xtask");
        self.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));
    }

    fn use_alpha2_validation(&mut self, head_commit: &str, requested_reuse: bool) {
        let argv = vec![
            "cargo".to_owned(),
            "test".to_owned(),
            "--locked".to_owned(),
            "-p".to_owned(),
            "xtask".to_owned(),
        ];
        let path = PathBuf::from(
            self.request["validation_evidence"][0]["result_path"]
                .as_str()
                .unwrap(),
        );
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": "yo.validation-run-summary/v1alpha2",
                "name": "xtask",
                "status": "passed",
                "exit_code": 0,
                "elapsed_seconds": 2,
                "log_bytes": 42,
                "log_path": ".local-exclude/validation-runs/xtask.log",
                "log_hash": format!("sha256:{}", "a".repeat(64)),
                "head_commit": head_commit,
                "worktree_state": "clean",
                "command_argv_count": argv.len(),
                "command_argv_hash": validation_summary::argv_hash(&argv),
                "reused": false,
                "reuse_policy": "reviewed-descendant/v1"
            }))
            .unwrap(),
        )
        .unwrap();
        self.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));
        self.request["validation_evidence"][0]["reused"] = json!(requested_reuse);
    }

    fn use_alpha3_validation(&mut self, head_commit: &str, requested_reuse: bool) {
        let argv = vec![
            "cargo".to_owned(),
            "test".to_owned(),
            "--locked".to_owned(),
            "-p".to_owned(),
            "xtask".to_owned(),
        ];
        let path = PathBuf::from(
            self.request["validation_evidence"][0]["result_path"]
                .as_str()
                .unwrap(),
        );
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": "yo.validation-run-summary/v1alpha3",
                "name": "xtask",
                "status": "passed",
                "exit_code": 0,
                "elapsed_seconds": 2,
                "log_bytes": 42,
                "log_path": ".local-exclude/validation-runs/xtask.log",
                "log_hash": format!("sha256:{}", "a".repeat(64)),
                "head_commit": head_commit,
                "worktree_state": "clean",
                "command_argv_count": argv.len(),
                "command_argv_hash": validation_summary::argv_hash(&argv),
                "reused": false,
                "reuse_policy": "reviewed-descendant-context/v1",
                "reuse_context": {
                    "schema": "yo.validation-reuse-context/v1alpha1",
                    "platform_os": env::consts::OS,
                    "platform_arch": env::consts::ARCH,
                    "toolchain_hash": validation_summary::current_toolchain_hash().unwrap(),
                    "external_state": "none-declared"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        self.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));
        self.request["validation_evidence"][0]["reused"] = json!(requested_reuse);
    }
}

// 이미 발행된 frozen v1 summary는 alpha 도입 뒤에도 같은 검증 의미로 받아
// 과거 Slice evidence를 마이그레이션하거나 다시 실행할 필요가 없게 한다.
#[test]
fn legacy_v1_validation_summary_remains_accepted() {
    let fixture = Fixture::new();

    assert_eq!(fixture.evaluate().unwrap().next_action, "integrate");
}

// v1alpha1 summary는 실행 당시 clean 후보와 exact argv를 자체적으로 결속하여
// coordinator가 다른 command를 선언해 기존 green 결과를 재사용하지 못하게 한다.
#[test]
fn alpha_validation_summary_binds_candidate_and_argv() {
    let mut fixture = Fixture::new();
    fixture.use_alpha_validation();
    assert_eq!(fixture.evaluate().unwrap().next_action, "integrate");

    fixture.request["validation_evidence"][0]["argv"][4] = json!("other-package");
    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("command_argv_hash does not match")
    );
}

// dirty worktree에서 나온 alpha summary는 command가 성공했더라도 clean 후보의
// exact validation으로 승격하지 않아 실행 중인 변경을 누락한 green을 막는다.
#[test]
fn alpha_validation_summary_rejects_a_dirty_launch() {
    let mut fixture = Fixture::new();
    fixture.use_alpha_validation();
    let path = PathBuf::from(
        fixture.request["validation_evidence"][0]["result_path"]
            .as_str()
            .unwrap(),
    );
    let mut summary: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    summary["worktree_state"] = json!("dirty");
    fs::write(&path, serde_json::to_vec(&summary).unwrap()).unwrap();
    fixture.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));

    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("worktree_state must be `clean`")
    );
}

// immutable review packet에서 이미 검증한 external-operation evidence도 gate가
// 같은 후보와 exact argv에 다시 결속하면 별도 summary 복사 없이 통합 증거가 된다.
#[test]
fn external_operation_evidence_binds_candidate_and_argv() {
    let mut fixture = Fixture::new();
    fixture.use_external_operation();
    assert_eq!(fixture.evaluate().unwrap().next_action, "integrate");

    fixture.request["validation_evidence"][0]["argv"][4] = json!("other-package");
    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("external-operation argv does not match")
    );
}

// external-operation evidence는 한 번 실행한 관측 결과이며 validation reuse 정책이
// 아니므로 gate request가 reused로 재분류하면 명시적으로 거부한다.
#[test]
fn external_operation_evidence_cannot_be_marked_reused() {
    let mut fixture = Fixture::new();
    fixture.use_external_operation();
    fixture.request["validation_evidence"][0]["reused"] = json!(true);
    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("external-operation evidence cannot be reused")
    );
}

// v1alpha2의 새 실행은 v1alpha1과 같은 exact-candidate 의미를 유지한다.
// 재사용 여부를 선언하지 않았는데 과거 HEAD를 끼워 넣을 수는 없다.
#[test]
fn alpha2_new_validation_still_binds_the_exact_candidate() {
    let mut fixture = Fixture::new();
    let candidate = fixture.candidate.clone();
    fixture.use_alpha2_validation(&candidate, false);
    assert_eq!(fixture.evaluate().unwrap().next_action, "integrate");

    let base = git_line(&fixture.repository.path, &["rev-parse", "HEAD^"]);
    fixture.use_alpha2_validation(&base, false);
    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("does not match candidate")
    );
}

// review-delta로 최종 후보까지 검토된 경우에만 과거 clean 통과 결과를 재사용하며,
// 원 실행 HEAD가 최종 후보의 Git 조상인지 trusted Git으로 증명한다.
#[test]
fn alpha2_reuses_a_passed_ancestor_validation() {
    let mut fixture = Fixture::new();
    let base = git_line(&fixture.repository.path, &["rev-parse", "HEAD^"]);
    fixture.use_alpha2_validation(&base, true);

    let result = fixture.evaluate().unwrap();
    assert_eq!(result.next_action, "integrate");
    assert!(result.validation[0].reused);
}

// v1alpha3 재사용은 Git ancestry와 exact argv뿐 아니라 gate 시점의 platform 및
// Rust toolchain context까지 같을 때만 열리고 context가 달라지면 즉시 닫힌다.
#[test]
fn alpha3_reuse_requires_the_current_execution_context() {
    let mut fixture = Fixture::new();
    let base = git_line(&fixture.repository.path, &["rev-parse", "HEAD^"]);
    fixture.use_alpha3_validation(&base, true);
    assert!(fixture.evaluate().unwrap().validation[0].reused);

    let path = PathBuf::from(
        fixture.request["validation_evidence"][0]["result_path"]
            .as_str()
            .unwrap(),
    );
    let mut summary: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    summary["reuse_context"]["platform_os"] = json!("changed-os");
    fs::write(&path, serde_json::to_vec(&summary).unwrap()).unwrap();
    fixture.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));
    assert!(fixture.evaluate().unwrap_err().contains("platform changed"));
}

// 분기만 같은 비조상 커밋의 green은 최종 후보에 포함된 코드 상태가 아니므로
// exact argv와 clean 상태가 같아도 재사용 증거로 승격하지 않는다.
#[test]
fn alpha2_rejects_non_ancestor_reuse() {
    let mut fixture = Fixture::new();
    fixture
        .repository
        .git(["switch", "--quiet", "--detach", "HEAD^"]);
    fixture
        .repository
        .write("tools/other.rs", "pub fn other() {}\n");
    fixture.repository.git(["add", "tools/other.rs"]);
    fixture
        .repository
        .git(["commit", "--quiet", "-m", "unrelated"]);
    let unrelated = git_line(&fixture.repository.path, &["rev-parse", "HEAD"]);
    fixture
        .repository
        .git(["switch", "--quiet", "slice/direct/gate-test"]);
    fixture.use_alpha2_validation(&unrelated, true);

    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("is not an ancestor")
    );
}

// summary는 실행 사실만 표현하므로 producer가 reused=true를 기록한 artifact나
// 실패 결과를 coordinator가 재사용했다고 선언한 경우 모두 닫힌다.
#[test]
fn alpha2_reuse_requires_an_executed_passing_summary() {
    let mut fixture = Fixture::new();
    let base = git_line(&fixture.repository.path, &["rev-parse", "HEAD^"]);
    fixture.use_alpha2_validation(&base, true);
    let path = PathBuf::from(
        fixture.request["validation_evidence"][0]["result_path"]
            .as_str()
            .unwrap(),
    );
    let mut summary: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    summary["reused"] = json!(true);
    fs::write(&path, serde_json::to_vec(&summary).unwrap()).unwrap();
    fixture.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));
    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("execution summary must record")
    );

    fixture.use_alpha2_validation(&base, true);
    let mut summary: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    summary["status"] = json!("failed");
    summary["exit_code"] = json!(1);
    fs::write(&path, serde_json::to_vec(&summary).unwrap()).unwrap();
    fixture.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));
    assert!(
        fixture
            .evaluate()
            .unwrap_err()
            .contains("only passed validation evidence can be reused")
    );
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
    fs::write(
        &path,
        br#"{"schema":"yo.validation-run-summary/v1","name":"xtask","status":"failed","exit_code":1,"elapsed_seconds":2,"log_bytes":42,"log_path":".local-exclude/validation-runs/xtask.log"}"#,
    )
    .unwrap();
    fixture.request["validation_evidence"][0]["result_hash"] = json!(digest_file(&path));

    assert_eq!(fixture.evaluate().unwrap().next_action, "validate");
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
    fs::write(path, b"changed after request\n").unwrap();

    assert!(fixture.evaluate().unwrap_err().contains("hash changed"));
}
