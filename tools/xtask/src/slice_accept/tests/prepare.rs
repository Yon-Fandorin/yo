use std::path::{Path, PathBuf};

use super::{
    PREPARE_REQUEST_SCHEMA, PREPARE_REQUEST_SCHEMA_V1_ALPHA2, ensure_distinct_paths,
    preflight_publication, prepare_with, require_unchanged, temporary_output_paths,
};
use crate::{
    review_protocol,
    slice_accept::{
        ACCEPT_REQUEST_SCHEMA, ACCEPT_REQUEST_SCHEMA_V1_ALPHA2, require_gate_authorization,
    },
    slice_close, slice_contract, test_support,
};

struct Fixture {
    repository: test_support::TestRepository,
    candidate: PathBuf,
    coordination: PathBuf,
    gate: PathBuf,
    prepare: PathBuf,
    candidate_commit: String,
}

impl Fixture {
    fn new() -> Self {
        let repository = test_support::TestRepository::new("post-gate-prepare");
        repository.write("README", "base\n");
        repository.git(["add", "README"]);
        repository.git(["commit", "--quiet", "-m", "base"]);
        let base = git_line(&repository.path, &["rev-parse", "HEAD"]);
        repository.git(["branch", "slice/direct/post-gate-test"]);
        let candidate = test_support::unique_path("post-gate-candidate");
        repository.git([
            "worktree",
            "add",
            "--quiet",
            candidate.to_str().unwrap(),
            "slice/direct/post-gate-test",
        ]);
        std::fs::create_dir_all(candidate.join("tools")).unwrap();
        std::fs::write(candidate.join("tools/example.rs"), "pub fn example() {}\n").unwrap();
        git(&candidate, &["add", "tools/example.rs"]);
        git(&candidate, &["commit", "--quiet", "-m", "candidate"]);
        let candidate_commit = git_line(&candidate, &["rev-parse", "HEAD"]);
        let diff = crate::git::trusted_output_bytes_in(
            &candidate,
            &[
                "diff",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-renames",
                &base,
                &candidate_commit,
                "--",
            ],
        )
        .unwrap();
        let diff_hash = review_protocol::digest(&diff);

        let coordination = repository
            .path
            .join(".local-exclude/coordination/post-gate-test");
        std::fs::write(
            repository.path.join(".git/info/exclude"),
            ".local-exclude/\n",
        )
        .unwrap();
        std::fs::create_dir_all(&coordination).unwrap();
        let contract = coordination.join("slice-contract.json");
        std::fs::write(
            &contract,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema": "yo.slice-contract/v1",
                    "slice": "post-gate-test",
                    "base": base,
                    "base_ref": "refs/heads/develop",
                    "owned_contracts": ["repository.workflow.post-gate.test"],
                    "dependencies": [],
                    "allowed_write_set": ["tools/**"],
                    "focused_checks": ["cargo test --locked -p xtask slice_accept"],
                    "slice_close_checks": ["cargo test --locked -p xtask"]
                }))
                .unwrap()
            ),
        )
        .unwrap();
        slice_contract::bind(&candidate, &contract).unwrap();

        let validation = coordination.join("validation.json");
        std::fs::write(
            &validation,
            br#"{"schema":"yo.validation-run-summary/v1","name":"xtask","status":"passed","exit_code":0,"elapsed_seconds":1,"log_bytes":10,"log_path":"validation.log"}"#,
        )
        .unwrap();
        let fresh = coordination.join("fresh.txt");
        std::fs::write(&fresh, b"fresh clear\n").unwrap();
        let quality = coordination.join("quality.txt");
        std::fs::write(&quality, b"quality clear\n").unwrap();
        let scope = super::super::effect_scope(
            "post-gate-test",
            &candidate_commit,
            "origin",
            "refs/heads/develop",
        );
        let gate = coordination.join("gate.json");
        std::fs::write(
            &gate,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema": "yo.slice-gate-request/v1alpha1",
                    "candidate_commit": candidate_commit,
                    "required_lenses": ["fresh-context", "code-quality"],
                    "validation_evidence": [{
                        "name": "xtask",
                        "argv": ["cargo", "test", "--locked", "-p", "xtask"],
                        "result_path": validation,
                        "result_hash": digest_file(&validation),
                        "candidate_commit": candidate_commit,
                        "reused": false
                    }],
                    "review_evidence": [{
                        "lens": "fresh-context",
                        "reviewer": "codex/fresh-session",
                        "route": "model-high/codex/gpt-5.6-sol/fresh-session",
                        "verdict": "clear",
                        "candidate_commit": candidate_commit,
                        "diff_hash": diff_hash,
                        "result_path": fresh,
                        "result_hash": digest_file(&fresh)
                    }, {
                        "lens": "code-quality",
                        "reviewer": "codex/quality-session",
                        "route": "model/codex/gpt-5.6-luna/quality-session",
                        "verdict": "clear",
                        "candidate_commit": candidate_commit,
                        "diff_hash": diff_hash,
                        "result_path": quality,
                        "result_hash": digest_file(&quality)
                    }],
                    "known_unverified_environments": [],
                    "risk": {
                        "classification": "human-attention",
                        "rationale": "changes workflow authority"
                    },
                    "approval": {
                        "kind": "exact_candidate",
                        "authority": "human/yon",
                        "scope": scope,
                        "candidate_commit": candidate_commit,
                        "diff_hash": diff_hash
                    }
                }))
                .unwrap()
            ),
        )
        .unwrap();
        let message = coordination.join("message.txt");
        std::fs::write(
            &message,
            b"feat(xtask): derive post-gate inputs\n\nDeveloper-Docs-Impact: none - test fixture\n",
        )
        .unwrap();
        let prepare = coordination.join("accept-prepare.json");
        std::fs::write(
            &prepare,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema": "yo.slice-accept-prepare-request/v1alpha1",
                    "gate_request_path": gate,
                    "message_source_path": message,
                    "close_observations": observations(),
                    "push_remote": "origin"
                }))
                .unwrap()
            ),
        )
        .unwrap();
        Self {
            repository,
            candidate,
            coordination,
            gate,
            prepare,
            candidate_commit,
        }
    }

    fn exact_alpha2() -> Self {
        let fixture = Self::new();
        fixture.set_prepare_schema(PREPARE_REQUEST_SCHEMA_V1_ALPHA2);
        fixture
    }

    fn standing_routine(prepare_schema: &str) -> Self {
        let fixture = Self::new();
        let mut gate: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&fixture.gate).unwrap()).unwrap();
        gate["risk"] = serde_json::json!({
            "classification": "routine",
            "rationale": "mechanical follow-through under an accepted contract"
        });
        gate["approval"] = serde_json::json!({
            "kind": "standing_routine",
            "authority": "human/yon",
            "scope": "routine exact-contract implementation"
        });
        std::fs::write(
            &fixture.gate,
            format!("{}\n", serde_json::to_string_pretty(&gate).unwrap()),
        )
        .unwrap();
        fixture.set_prepare_schema(prepare_schema);
        fixture
    }

    fn set_prepare_schema(&self, schema: &str) {
        let mut prepare: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&self.prepare).unwrap()).unwrap();
        prepare["schema"] = serde_json::Value::String(schema.to_owned());
        std::fs::write(
            &self.prepare,
            format!("{}\n", serde_json::to_string_pretty(&prepare).unwrap()),
        )
        .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = crate::git::command_in(&self.repository.path, false)
            .args(["worktree", "remove", "--force", "--"])
            .arg(&self.candidate)
            .status();
    }
}

fn observations() -> slice_close::CloseObservations {
    serde_json::from_value(serde_json::json!({
        "execution_lanes": [{
            "lane": "integration",
            "mode": "serial",
            "operation_count": 1,
            "max_concurrency": 1
        }],
        "review": {
            "rounds": 1,
            "findings": {
                "reported": 1,
                "resolved": 1,
                "not_reproduced": 0,
                "accepted_limits": 0,
                "remaining": 0
            }
        },
        "review_packets": {
            "publication_count": 1,
            "total_managed_tokens": 1200,
            "largest_sections": [{
                "kind": "GitDiff",
                "name": "candidate.diff",
                "rendered_bytes": 4000,
                "rendered_tokens": 1000
            }],
            "reused_inputs": ["repository authority"]
        },
        "unverified_validation": [],
        "elapsed_bottleneck": {
            "name": "review",
            "elapsed_milliseconds": 2500
        }
    }))
    .unwrap()
}

// 실제 worktree registry와 ready gate를 통과한 한 번의 prepare가 gate·message·close
// 바이트 해시를 계산해 두 downstream request를 같은 candidate에 결속하는지 확인합니다.
#[test]
fn one_prepare_derives_hash_bound_accept_and_close_requests() {
    let fixture = Fixture::new();

    prepare_with(&fixture.candidate, &fixture.prepare, || Ok(())).unwrap();

    let accept_path = fixture.coordination.join("accept.json");
    let close_path = fixture.coordination.join("close-prepare.json");
    let accept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&accept_path).unwrap()).unwrap();
    let close: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&close_path).unwrap()).unwrap();
    assert_eq!(accept["slice"], "post-gate-test");
    assert_eq!(accept["push"]["reference"], "refs/heads/develop");
    assert_eq!(accept["gate_request_hash"], digest_file(&fixture.gate));
    assert_eq!(
        accept["close_prepare_request_hash"],
        digest_file(&close_path)
    );
    assert_eq!(close["slice"], "post-gate-test");
    assert_eq!(close["review"]["rounds"], 1);
    assert!(
        accept["approval_scope"]
            .as_str()
            .unwrap()
            .contains(&fixture.candidate_commit)
    );
}

// v1alpha2 준비는 satisfied standing_routine을 새 사람 승인으로 가장하지 않고,
// 후보·remote·ref·squash·close를 별도 effect_scope에 결정적으로 결속합니다.
#[test]
fn alpha2_standing_routine_derives_exact_effect_scope() {
    let fixture = Fixture::standing_routine(PREPARE_REQUEST_SCHEMA_V1_ALPHA2);

    prepare_with(&fixture.candidate, &fixture.prepare, || Ok(())).unwrap();

    let accept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.coordination.join("accept.json")).unwrap())
            .unwrap();
    assert_eq!(accept["schema"], ACCEPT_REQUEST_SCHEMA_V1_ALPHA2);
    assert!(accept.get("approval_scope").is_none());
    assert_eq!(
        accept["effect_scope"],
        super::super::effect_scope(
            "post-gate-test",
            &fixture.candidate_commit,
            "origin",
            "refs/heads/develop"
        )
    );
}

// 새 준비 버전은 exact_candidate 게이트도 같은 exact effect 결속으로 옮겨 기존
// 강한 승인 경로를 standing_routine 지원 때문에 약화시키지 않습니다.
#[test]
fn alpha2_exact_candidate_uses_the_same_effect_scope() {
    let fixture = Fixture::exact_alpha2();

    prepare_with(&fixture.candidate, &fixture.prepare, || Ok(())).unwrap();

    let accept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.coordination.join("accept.json")).unwrap())
            .unwrap();
    assert_eq!(accept["schema"], ACCEPT_REQUEST_SCHEMA_V1_ALPHA2);
    assert!(
        accept["effect_scope"]
            .as_str()
            .unwrap()
            .contains(&fixture.candidate_commit)
    );
}

// 이미 발행된 v1alpha1 의미는 그대로 exact_candidate 전용이므로 같은 standing
// 게이트가 새 버전 선택 없이 조용히 허용되지 않습니다.
#[test]
fn alpha1_standing_routine_remains_rejected() {
    let fixture = Fixture::standing_routine(PREPARE_REQUEST_SCHEMA);

    let error = prepare_with(&fixture.candidate, &fixture.prepare, || Ok(())).unwrap_err();

    assert!(error.contains("v1alpha1 requires"));
    assert!(!fixture.coordination.join("accept.json").exists());
    assert!(!fixture.coordination.join("close-prepare.json").exists());
}

// 준비 도중 gate 파일이 같은 의미의 JSON이라도 바이트가 바뀌면 원래 capture의
// hash를 가진 accept/close request를 하나도 발행하지 않고 stale 상태로 중단합니다.
#[test]
fn stale_gate_change_publishes_no_downstream_artifact() {
    let fixture = Fixture::new();
    let gate = fixture.gate.clone();

    let error = prepare_with(&fixture.candidate, &fixture.prepare, || {
        let mut bytes = std::fs::read(&gate).unwrap();
        bytes.push(b'\n');
        std::fs::write(&gate, bytes).unwrap();
        Ok(())
    })
    .unwrap_err();

    assert!(error.contains("Slice gate request changed"));
    assert!(!fixture.coordination.join("accept.json").exists());
    assert!(!fixture.coordination.join("close-prepare.json").exists());
}

// 새 준비기는 close 관측값만 받아 기존 v1alpha1 close request의 평평한 JSON 모양을
// 그대로 생성하므로 이미 고정된 소비자가 새 중첩 필드를 알 필요가 없습니다.
#[test]
fn close_observations_generate_the_frozen_downstream_shape() {
    let bytes = slice_close::close_prepare_request_bytes(
        "sample",
        ".local-exclude/coordination/sample/gate.json",
        &observations(),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(value["schema"], "yo.slice-close-prepare-request/v1alpha1");
    assert_eq!(value["slice"], "sample");
    assert_eq!(value["execution_lanes"][0]["lane"], "integration");
    assert!(value.get("observations").is_none());
}

// 승인 scope는 remote까지 포함한 exact effect와 같아야 하며, 다른 remote나 후보의
// 문자열은 downstream request가 발행되기 전에 gate 결속 검사에서 거부됩니다.
#[test]
fn gate_scope_mismatch_is_rejected_before_publication() {
    let path = test_support::unique_path("post-gate-approval-scope");
    std::fs::write(
        &path,
        br#"{"approval":{"kind":"exact_candidate","scope":"scope-a"}}"#,
    )
    .unwrap();

    require_gate_authorization(&path, "scope-a", ACCEPT_REQUEST_SCHEMA).unwrap();
    let error = require_gate_authorization(&path, "scope-b", ACCEPT_REQUEST_SCHEMA).unwrap_err();

    assert!(error.contains("exact_candidate approval"));
    std::fs::remove_file(path).unwrap();
}

// 첫 capture 뒤 gate나 의미 원문의 바이트 하나라도 달라지면 새 hash로 조용히
// 발행하지 않고 stale 입력으로 중단하는 공통 비교 경계를 확인합니다.
#[test]
fn changed_gate_bytes_fail_closed() {
    require_unchanged(b"gate-v1\n", b"gate-v1\n", "Slice gate request").unwrap();
    let error = require_unchanged(b"gate-v1\n", b"gate-v2\n", "Slice gate request").unwrap_err();

    assert_eq!(
        error,
        "Slice gate request changed before post-gate publication"
    );
}

// accept와 close request 중 하나에 다른 과거 바이트가 있으면 첫 파일도 쓰기 전에
// preflight가 멈춰 서로 다른 준비 시도의 artifact가 한 세트처럼 보이지 않습니다.
#[test]
fn existing_mismatched_output_fails_before_partial_publication() {
    let path = test_support::unique_path("post-gate-existing-output");
    std::fs::write(&path, b"old\n").unwrap();

    let error = preflight_publication(&path, b"new\n", "Slice accept request").unwrap_err();

    assert!(error.contains("existing Slice accept request changed"));
    assert_eq!(std::fs::read(&path).unwrap(), b"old\n");
    std::fs::remove_file(path).unwrap();
}

// candidate prefix를 포함한 임시 경로는 같은 exact 후보에는 재사용 가능하고 다른
// 후보에는 분리되어 실패 재개 시 이전 plan/message를 우연히 소비하지 않습니다.
#[test]
fn temporary_effect_paths_are_candidate_scoped() {
    let first = temporary_output_paths("sample", &"a".repeat(40));
    let same = temporary_output_paths("sample", &"a".repeat(40));
    let other = temporary_output_paths("sample", &"b".repeat(40));

    assert_eq!(first, same);
    assert_ne!(first, other);
    assert!(first.0.to_string_lossy().contains("aaaaaaaaaaaa"));
}

// 준비 입력과 생성 artifact가 같은 경로를 가리키면 원문을 출력으로 덮을 수 있으므로
// 모든 역할이 서로 다른 경로인지 publication 전에 검사합니다.
#[test]
fn input_and_output_path_aliases_are_rejected() {
    let paths = [
        Path::new("request"),
        Path::new("gate"),
        Path::new("message"),
        Path::new("accept"),
        Path::new("close"),
        Path::new("message-out"),
        Path::new("plan"),
    ];
    ensure_distinct_paths(
        paths[0], paths[1], paths[2], paths[3], paths[4], paths[5], paths[6],
    )
    .unwrap();
    let error = ensure_distinct_paths(
        paths[0], paths[1], paths[2], paths[0], paths[4], paths[5], paths[6],
    )
    .unwrap_err();

    assert!(error.contains("must be distinct"));
}

fn git(repository: &Path, arguments: &[&str]) {
    let status = crate::git::command_in(repository, false)
        .args(arguments)
        .status()
        .unwrap();
    assert!(status.success());
}

fn git_line(repository: &Path, arguments: &[&str]) -> String {
    crate::git::output_in(repository, arguments, false)
        .unwrap()
        .trim()
        .to_owned()
}

fn digest_file(path: &Path) -> String {
    review_protocol::digest(&std::fs::read(path).unwrap())
}
