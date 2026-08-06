mod apply;
mod environment;
mod plan;
mod storage;

use std::path::{Path, PathBuf};

use super::{Plan, build_plan};
use crate::{slice_contract, test_support};

struct CloseFixture {
    repository: test_support::TestRepository,
    slice_worktree: PathBuf,
    contract_path: PathBuf,
    plan_path: PathBuf,
}

impl CloseFixture {
    fn new() -> Self {
        Self::new_for("refs/heads/develop", "slice/direct/sample")
    }

    fn new_wave() -> Self {
        Self::new_for("refs/heads/wave/w1", "slice/w1/sample")
    }

    fn new_for(integration_ref: &str, slice_branch: &str) -> Self {
        let repository = test_support::TestRepository::new("slice-close");
        repository.write("base.txt", "base\n");
        repository.git(["add", "base.txt"]);
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

        let contract_path = test_support::unique_path("slice-close-contract.json");
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

        Self {
            repository,
            slice_worktree,
            contract_path,
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
