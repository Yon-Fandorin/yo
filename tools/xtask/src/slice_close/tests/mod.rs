mod apply;
mod environment;
mod metrics;
mod plan;
mod storage;

use std::path::{Path, PathBuf};

use super::{Plan, build_plan};
use crate::{slice_contract, test_support};

struct CloseFixture {
    repository: test_support::TestRepository,
    slice_worktree: PathBuf,
    contract_path: PathBuf,
    metrics_path: PathBuf,
    plan_path: PathBuf,
}

impl CloseFixture {
    fn new() -> Self {
        Self::new_for("refs/heads/develop", "slice/direct/sample", false)
    }

    fn new_wave() -> Self {
        Self::new_for("refs/heads/wave/w1", "slice/w1/sample", false)
    }

    fn new_after_metrics_cutover() -> Self {
        Self::new_for("refs/heads/develop", "slice/direct/sample", true)
    }

    fn new_for(integration_ref: &str, slice_branch: &str, after_metrics_cutover: bool) -> Self {
        let repository = test_support::TestRepository::new("slice-close");
        std::fs::write(
            repository.path.join(".git/info/exclude"),
            ".local-exclude/\n",
        )
        .unwrap();
        repository.write("base.txt", "base\n");
        repository.git(["add", "base.txt"]);
        if after_metrics_cutover {
            repository.write(
                "tools/xtask/src/slice_close/metrics-cutover",
                "yo.slice-close-metrics/v1\n",
            );
            repository.git(["add", "tools/xtask/src/slice_close/metrics-cutover"]);
        }
        repository.git(["commit", "--quiet", "-m", "test: base"]);
        let base = output(&repository.path, &["rev-parse", "HEAD"]);
        if integration_ref != "refs/heads/develop" {
            repository.git(["switch", "--quiet", "-c", "wave/w1"]);
        }
        repository.git(["branch", slice_branch]);

        let slice_worktree = test_support::unique_path("slice-close-worktree");
        repository.git([
            "worktree",
            "add",
            "--quiet",
            slice_worktree.to_str().unwrap(),
            slice_branch,
        ]);
        let slice_worktree = std::fs::canonicalize(slice_worktree).unwrap();
        std::fs::write(slice_worktree.join("feature.txt"), "accepted change\n").unwrap();
        git(&slice_worktree, &["add", "feature.txt"]);
        git(
            &slice_worktree,
            &["commit", "--quiet", "-m", "feat: Slice candidate"],
        );

        repository.write("feature.txt", "accepted change\n");
        repository.git(["add", "feature.txt"]);
        let accepted_message = if integration_ref == "refs/heads/develop" {
            "feat: accepted Slice\n\nSlice-Review: fresh-context - completed - codex/test - clear"
        } else {
            "feat: accepted Wave Slice\n\nSlice-Review: fresh-context - completed - codex/test - clear\nSlice-Review: integration - completed - codex/test - clear"
        };
        repository.git(["commit", "--quiet", "-m", accepted_message]);

        let contract_directory = repository.path.join(".local-exclude/coordination/sample");
        std::fs::create_dir_all(&contract_directory).unwrap();
        let contract_path = contract_directory.join("slice-contract.json");
        std::fs::write(
            &contract_path,
            format!(
                r#"{{
  "schema": "yo.slice-contract/v1",
  "slice": "sample",
  "base": "{base}",
  "base_ref": "{integration_ref}",
  "owned_contracts": ["test.sample"],
  "dependencies": [],
  "allowed_write_set": ["feature.txt"],
  "focused_checks": ["test focused"],
  "slice_close_checks": ["test close"]
}}"#
            ),
        )
        .unwrap();
        slice_contract::bind(&slice_worktree, &contract_path).unwrap();

        let slice_candidate = output(&slice_worktree, &["rev-parse", "HEAD"]);
        let accepted_commit = output(&repository.path, &["rev-parse", "HEAD"]);
        let metrics_path = contract_directory.join("close-metrics.json");
        std::fs::write(
            &metrics_path,
            close_metrics(&slice_candidate, &accepted_commit),
        )
        .unwrap();

        Self {
            repository,
            slice_worktree,
            contract_path,
            metrics_path,
            plan_path: test_support::unique_path("slice-close-plan.json"),
        }
    }

    fn plan(&self) -> Plan {
        build_plan(&self.repository.path, "sample").unwrap()
    }

    fn write_plan(&self, plan: &Plan) {
        std::fs::write(&self.plan_path, serde_json::to_vec_pretty(plan).unwrap()).unwrap();
    }

    fn commit_later(&self, relative: &str) {
        self.repository.write(relative, "later accepted change\n");
        self.repository.git(["add", relative]);
        self.repository.git([
            "commit",
            "--quiet",
            "-m",
            "feat: later Slice\n\nSlice-Review: fresh-context - completed - codex/test - clear",
        ]);
    }
}

fn close_metrics(slice_candidate: &str, accepted_commit: &str) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "schema": "yo.slice-close-metrics/v1",
        "slice": "sample",
        "slice_candidate": slice_candidate,
        "accepted_commit": accepted_commit,
        "execution_lanes": [
            {
                "lane": "cargo_validation",
                "mode": "serial",
                "operation_count": 2,
                "max_concurrency": 1
            },
            {
                "lane": "integration",
                "mode": "serial",
                "operation_count": 1,
                "max_concurrency": 1
            }
        ],
        "review": {
            "rounds": 1,
            "findings": {
                "reported": 0,
                "resolved": 0,
                "not_reproduced": 0,
                "accepted_limits": 0,
                "remaining": 0
            }
        },
        "review_packets": {
            "publication_count": 1,
            "total_managed_tokens": 100,
            "largest_sections": [{
                "kind": "git_diff",
                "name": "base-to-candidate",
                "rendered_bytes": 200,
                "rendered_tokens": 50
            }],
            "reused_inputs": []
        },
        "validation": [{
            "name": "focused",
            "argv": ["cargo", "test", "--locked", "-p", "xtask"],
            "runs": 1,
            "status": "passed",
            "reused": false
        }],
        "elapsed_bottleneck": {
            "name": "full validation",
            "elapsed_milliseconds": 1000
        },
        "known_unverified_environments": []
    }))
    .unwrap();
    bytes.push(b'\n');
    bytes
}

impl Drop for CloseFixture {
    fn drop(&mut self) {
        let registered = crate::git::output_in(
            &self.repository.path,
            &["worktree", "list", "--porcelain"],
            false,
        )
        .is_ok_and(|listing| {
            listing.lines().any(|line| {
                line.strip_prefix("worktree ")
                    .is_some_and(|path| Path::new(path) == self.slice_worktree)
            })
        });
        if registered {
            let _ = crate::git::command_in(&self.repository.path, false)
                .args(["worktree", "remove", "--force", "--"])
                .arg(&self.slice_worktree)
                .status();
        }
        let _ = std::fs::remove_file(&self.plan_path);
        let _ = std::fs::remove_file(&self.contract_path);
        let _ = std::fs::remove_dir_all(self.repository.path.join(".local-exclude"));
        let _ = std::fs::remove_dir_all(&self.slice_worktree);
    }
}

fn output(repository: &Path, arguments: &[&str]) -> String {
    let output = crate::git::command_in(repository, false)
        .args(arguments)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn git(repository: &Path, arguments: &[&str]) {
    assert!(
        crate::git::command_in(repository, false)
            .args(arguments)
            .status()
            .unwrap()
            .success()
    );
}

fn git_succeeds(repository: &Path, arguments: &[&str]) -> bool {
    crate::git::command_in(repository, false)
        .args(arguments)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}
