use std::path::{Path, PathBuf};

use super::super::{model::ResultRecord, prepare};
use crate::test_support;

pub(super) struct Fixture {
    pub(super) repository: test_support::TestRepository,
    pub(super) request: PathBuf,
    pub(super) slice: String,
}

impl Fixture {
    pub(super) fn new(label: &str) -> Self {
        let repository = test_support::TestRepository::new(label);
        repository.write("base.txt", "base\n");
        repository.git(["add", "base.txt"]);
        repository.git(["commit", "--quiet", "-m", "test: base"]);
        std::fs::write(
            repository.path.join(".git/info/exclude"),
            ".local-exclude/\n",
        )
        .unwrap();
        let slice = format!("{label}-activation");
        let request = test_support::unique_path("activation-slice-request.json");
        std::fs::write(
            &request,
            format!(
                r#"{{
  "schema": "yo.activation-slice-request/v1",
  "slice": "{slice}",
  "owned_contracts": ["test.activation"],
  "dependencies": ["approved test revision"]
}}
"#
            ),
        )
        .unwrap();
        Self {
            repository,
            request,
            slice,
        }
    }

    pub(super) fn prepare(&self) -> ResultRecord {
        prepare(&self.repository.path, &self.request).unwrap()
    }

    pub(super) fn worktree(&self) -> PathBuf {
        self.repository
            .path
            .join(".local-exclude/worktrees")
            .join(&self.slice)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let worktree = self.worktree();
        if worktree.exists() {
            let _ = crate::git::command_in(&self.repository.path, false)
                .args(["worktree", "remove", "--force", "--"])
                .arg(&worktree)
                .status();
        }
        let _ = std::fs::remove_file(&self.request);
    }
}

pub(super) fn output(repository: &Path, arguments: &[&str]) -> String {
    crate::git::output_in(repository, arguments, false)
        .unwrap()
        .trim()
        .to_owned()
}
