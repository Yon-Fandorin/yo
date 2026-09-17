use std::{path::Path, process::Stdio};

use crate::{git, slice_worktree};

pub(super) fn recover_precommit_failure(
    integration: &Path,
    integration_ref: &str,
    integration_head: &str,
    candidate_diff: &[u8],
    changed_paths: &[String],
    error: String,
) -> String {
    match restore_exact_squash(
        integration,
        integration_ref,
        integration_head,
        candidate_diff,
        changed_paths,
    ) {
        Ok(()) => format!(
            "{error}; the exact staged squash was automatically restored and the integration worktree is clean"
        ),
        Err(restore) => format!(
            "{error}; automatic pre-commit restoration was not safe: {restore}; inspect the integration worktree"
        ),
    }
}

fn restore_exact_squash(
    integration: &Path,
    integration_ref: &str,
    integration_head: &str,
    candidate_diff: &[u8],
    changed_paths: &[String],
) -> Result<(), String> {
    slice_worktree::expect_ref(integration, integration_ref, integration_head)?;
    let staged = git::output_bytes_in(
        integration,
        &[
            "diff",
            "--cached",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            "--",
        ],
        false,
    )?;
    if staged != candidate_diff {
        return Err("staged bytes no longer equal the exact candidate diff".to_owned());
    }
    let status = git::command_in(integration, false)
        .args(["restore", "--source=HEAD", "--staged", "--worktree", "--"])
        .args(changed_paths)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("cannot start exact staged-squash restoration: {error}"))?;
    if !status.success() {
        return Err(format!("exact staged-squash restoration failed ({status})"));
    }
    slice_worktree::ensure_clean(
        integration,
        "integration worktree",
        "Slice acceptance rollback",
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Stdio,
    };

    use crate::{
        git,
        test_support::{self, TestRepository},
    };

    struct AcceptanceFixture {
        repository: TestRepository,
        candidate: PathBuf,
        integration_head: String,
    }

    impl AcceptanceFixture {
        fn new(label: &str) -> Self {
            let repository = TestRepository::new(label);
            repository.write("base.txt", "base\n");
            repository.git(["add", "base.txt"]);
            repository.git(["commit", "--quiet", "-m", "base"]);
            let integration_head = output(&repository.path, &["rev-parse", "HEAD"]);
            repository.git(["branch", "slice/direct/example"]);
            let candidate = test_support::unique_path(label);
            repository.git([
                "worktree",
                "add",
                "--quiet",
                candidate.to_str().unwrap(),
                "slice/direct/example",
            ]);
            let changed = candidate.join("tools/example.rs");
            fs::create_dir_all(changed.parent().unwrap()).unwrap();
            fs::write(&changed, "pub fn accepted() {}\n").unwrap();
            git_cmd(&candidate, &["add", "tools/example.rs"]);
            git_cmd(&candidate, &["commit", "--quiet", "-m", "candidate"]);
            Self {
                repository,
                candidate,
                integration_head,
            }
        }

        fn stage_candidate(&self) -> Vec<u8> {
            let status = git::command_in(&self.repository.path, false)
                .args(["merge", "--squash", "slice/direct/example"])
                .stdin(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
            git::output_bytes_in(
                &self.repository.path,
                &[
                    "diff",
                    "--cached",
                    "--binary",
                    "--full-index",
                    "--no-ext-diff",
                    "--no-renames",
                    "--",
                ],
                false,
            )
            .unwrap()
        }
    }

    impl Drop for AcceptanceFixture {
        fn drop(&mut self) {
            let _ = git::command_in(&self.repository.path, false)
                .args(["worktree", "remove", "--force", "--"])
                .arg(&self.candidate)
                .status();
        }
    }

    fn output(repository: &Path, arguments: &[&str]) -> String {
        git::output_in(repository, arguments, false)
            .unwrap()
            .trim()
            .to_owned()
    }

    fn git_cmd(repository: &Path, arguments: &[&str]) {
        assert!(
            git::command_in(repository, false)
                .args(arguments)
                .status()
                .unwrap()
                .success()
        );
    }

    // exact squash 뒤 hook이나 commit 실행이 실패하면 원래 integration HEAD와 candidate
    // diff가 그대로인 경우에만 자동 restore하여 다음 accept가 수동 정리 없이 재개됩니다.
    #[test]
    fn precommit_failure_restores_the_exact_squash() {
        let fixture = AcceptanceFixture::new("slice-accept-rollback");
        let candidate_diff = fixture.stage_candidate();
        let error = super::recover_precommit_failure(
            &fixture.repository.path,
            "refs/heads/develop",
            &fixture.integration_head,
            &candidate_diff,
            &["tools/example.rs".to_owned()],
            "synthetic commit failure".to_owned(),
        );

        assert!(error.contains("synthetic commit failure"));
        assert!(error.contains("automatically restored"));
        assert_eq!(
            output(&fixture.repository.path, &["rev-parse", "HEAD"]),
            fixture.integration_head
        );
        assert!(
            git::output_bytes_in(
                &fixture.repository.path,
                &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
                false,
            )
            .unwrap()
            .is_empty()
        );
    }
}
