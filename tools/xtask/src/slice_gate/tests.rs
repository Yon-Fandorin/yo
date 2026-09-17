use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use super::evaluate;
use crate::{git, review_protocol, slice_contract, test_support};

mod admission;
mod evidence;
mod revalidation;
mod trailers;

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
        let diff = git::trusted_output_bytes_in(
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
        fs::create_dir_all(&artifacts).unwrap();
        let contract = artifacts.join("slice-contract.json");
        fs::write(
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
        fs::write(
            &validation,
            br#"{"schema":"yo.validation-run-summary/v1","name":"xtask","status":"passed","exit_code":0,"elapsed_seconds":2,"log_bytes":42,"log_path":".local-exclude/validation-runs/xtask.log"}"#,
        )
        .unwrap();
        let validation_hash = digest_file(&validation);
        let fresh = artifacts.join("fresh.txt");
        fs::write(&fresh, b"fresh review clear\n").unwrap();
        let fresh_hash = digest_file(&fresh);
        let quality = artifacts.join("quality.txt");
        fs::write(&quality, b"quality review clear\n").unwrap();
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
        fs::write(&request_path, serde_json::to_vec(&self.request).unwrap()).unwrap();
        evaluate(&self.repository.path, &request_path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.artifacts);
    }
}

fn digest_file(path: &Path) -> String {
    review_protocol::digest(&fs::read(path).unwrap())
}

fn git_line(repository: &Path, arguments: &[&str]) -> String {
    git::trusted_output_in(repository, arguments)
        .unwrap()
        .trim()
        .to_owned()
}
