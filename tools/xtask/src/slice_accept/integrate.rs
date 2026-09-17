use std::{
    path::{Path, PathBuf},
    process::Stdio,
    str,
};

use super::{COMMIT_CANDIDATE_HK_RECEIPT, COMMIT_GIT_HOOKS};
use crate::{
    git,
    impact::{self, ImpactInput},
    slice_worktree,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn integrate_candidate(
    integration: &Path,
    integration_ref: &str,
    integration_head: &str,
    candidate_repository: &Path,
    candidate_branch: &str,
    candidate_base: &str,
    candidate_head: &str,
    message_output: &Path,
    commit_verification: &str,
) -> Result<String, String> {
    let commit = match commit_verification {
        COMMIT_GIT_HOOKS => impact::review_coverage::create_accepted_commit,
        COMMIT_CANDIDATE_HK_RECEIPT => {
            impact::review_coverage::create_accepted_commit_from_verified_candidate
        },
        _ => return Err("unsupported accepted commit verification mode".to_owned()),
    };
    integrate_candidate_with(
        integration,
        integration_ref,
        integration_head,
        candidate_repository,
        candidate_branch,
        candidate_base,
        candidate_head,
        message_output,
        commit,
    )
}

#[allow(clippy::too_many_arguments)]
fn integrate_candidate_with(
    integration: &Path,
    integration_ref: &str,
    integration_head: &str,
    candidate_repository: &Path,
    candidate_branch: &str,
    candidate_base: &str,
    candidate_head: &str,
    message_output: &Path,
    commit: impl FnOnce(&Path, &Path) -> Result<(), String>,
) -> Result<String, String> {
    let candidate_diff = canonical_diff(candidate_repository, candidate_base, candidate_head)?;
    let changed_paths =
        candidate_changed_paths(candidate_repository, candidate_base, candidate_head)?;
    let branch = integration_ref
        .strip_prefix("refs/heads/")
        .ok_or_else(|| format!("unsupported integration ref `{integration_ref}`"))?;
    let preflight = ImpactInput {
        message: git::read(message_output, "prepared accepted commit message")?,
        changed_paths: changed_paths.clone(),
        branch: branch.to_owned(),
        merge_head: None,
        repository: integration.to_path_buf(),
        inherit_git_environment: false,
    };
    impact::preflight::check_candidate(&preflight, &candidate_diff)?;
    slice_worktree::ensure_clean(integration, "integration worktree", "Slice acceptance")?;
    slice_worktree::expect_ref(integration, integration_ref, integration_head)?;
    slice_worktree::expect_ref(candidate_repository, candidate_branch, candidate_head)?;

    let merge_status = git::command_in(integration, false)
        .args(["merge", "--squash", candidate_branch])
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("cannot start accepted Slice squash: {error}"))?;
    if !merge_status.success() {
        return Err(format!(
            "accepted Slice squash failed ({merge_status}); integration worktree requires inspection"
        ));
    }
    let staged_diff = git::output_bytes_in(
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
    if staged_diff != candidate_diff {
        return Err(
            "squashed index differs from the exact reviewed candidate diff; integration worktree requires inspection"
                .to_owned(),
        );
    }

    let commit_result = (|| {
        let input = ImpactInput::load_from(
            integration,
            message_output.to_path_buf(),
            None,
            Some(branch.to_owned()),
            true,
        )?;
        impact::preflight::check(&input)?;
        commit(&input.repository, message_output)
    })();
    if let Err(error) = commit_result {
        return Err(super::rollback::recover_precommit_failure(
            integration,
            integration_ref,
            integration_head,
            &candidate_diff,
            &changed_paths,
            error,
        ));
    }
    let accepted_commit = slice_worktree::resolve_commit(integration, integration_ref)?;
    let accepted_diff = canonical_diff(
        integration,
        &format!("{accepted_commit}^"),
        &accepted_commit,
    )?;
    if accepted_diff != candidate_diff {
        return Err("accepted commit differs from the exact reviewed candidate diff".to_owned());
    }
    Ok(accepted_commit)
}

pub(super) fn integration_worktree(repository: &Path, reference: &str) -> Result<PathBuf, String> {
    let matches = slice_worktree::worktrees(repository)?
        .into_iter()
        .filter(|worktree| worktree.branch.as_deref() == Some(reference))
        .map(|worktree| worktree.path)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(format!(
            "no registered integration worktree has branch `{reference}`"
        )),
        _ => Err(format!(
            "multiple registered integration worktrees have branch `{reference}`"
        )),
    }
}

fn candidate_changed_paths(
    repository: &Path,
    base: &str,
    candidate: &str,
) -> Result<Vec<String>, String> {
    let output = git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--name-only",
            "-z",
            "--diff-filter=ACDMR",
            "--no-renames",
            base,
            candidate,
            "--",
        ],
    )?;
    let paths = output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            str::from_utf8(path)
                .map(str::to_owned)
                .map_err(|error| format!("candidate changed path must be UTF-8: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Err("accepted candidate has no changed paths".to_owned());
    }
    Ok(paths)
}

fn canonical_diff(repository: &Path, base: &str, candidate: &str) -> Result<Vec<u8>, String> {
    git::trusted_output_bytes_in(
        repository,
        &[
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            base,
            candidate,
            "--",
        ],
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use super::{canonical_diff, integration_worktree};
    use crate::{
        git, slice_worktree,
        test_support::{self, TestRepository},
    };

    struct AcceptanceFixture {
        repository: TestRepository,
        candidate: PathBuf,
        message: PathBuf,
        integration_head: String,
        candidate_head: String,
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
            let candidate_head = output(&candidate, &["rev-parse", "HEAD"]);
            let message = test_support::unique_path(&format!("{label}-message"));
            Self {
                repository,
                candidate,
                message,
                integration_head,
                candidate_head,
            }
        }

        fn write_message(&self, docs_impact: &str) {
            fs::write(
                &self.message,
                format!(
                    "feat: accepted candidate\n\n\
                     Developer-Docs-Impact: {docs_impact}\n\
                     Slice-Review: fresh-context - completed - human/yon - clear\n\
                     Slice-Review: code-quality - completed - human/yon - clear\n"
                ),
            )
            .unwrap();
        }

        fn integrate(
            &self,
            commit: impl FnOnce(&Path, &Path) -> Result<(), String>,
        ) -> Result<String, String> {
            super::integrate_candidate_with(
                &self.repository.path,
                "refs/heads/develop",
                &self.integration_head,
                &self.candidate,
                "refs/heads/slice/direct/example",
                &self.integration_head,
                &self.candidate_head,
                &self.message,
                commit,
            )
        }
    }

    impl Drop for AcceptanceFixture {
        fn drop(&mut self) {
            let _ = git::command_in(&self.repository.path, false)
                .args(["worktree", "remove", "--force", "--"])
                .arg(&self.candidate)
                .status();
            let _ = fs::remove_file(&self.message);
        }
    }

    fn output(repository: &Path, arguments: &[&str]) -> String {
        git::output_in(repository, arguments, false)
            .unwrap()
            .trim()
            .to_owned()
    }

    fn status(repository: &Path) -> Vec<u8> {
        git::output_bytes_in(
            repository,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            false,
        )
        .unwrap()
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

    // accept는 호출한 Slice worktree를 integration으로 재사용하지 않고 공용 worktree
    // registry에서 계약의 full branch ref를 가진 유일한 worktree를 선택합니다.
    #[test]
    fn integration_worktree_is_selected_by_full_branch_ref() {
        let repository = TestRepository::new("slice-accept-integration-worktree");
        repository.write("tracked.txt", "base\n");
        repository.git(["add", "tracked.txt"]);
        repository.git(["commit", "--quiet", "-m", "base"]);
        repository.git(["switch", "-q", "-c", "slice/direct/example"]);
        let integration = repository.path.join("develop-integration");
        repository.git([
            "worktree",
            "add",
            "-q",
            integration.to_str().unwrap(),
            "develop",
        ]);

        assert_eq!(
            integration_worktree(&repository.path, "refs/heads/develop").unwrap(),
            integration
        );
        assert!(
            integration_worktree(&repository.path, "refs/heads/missing")
                .unwrap_err()
                .contains("no registered integration worktree")
        );

        let listed = slice_worktree::worktrees(&repository.path).unwrap();
        assert!(
            listed
                .iter()
                .any(|worktree| worktree.branch.as_deref() == Some("refs/heads/develop"))
        );
    }

    // Developer Docs trailer처럼 staged path만 필요하던 검사를 실제 squash 전에 수행해
    // 잘못된 메시지가 integration index를 더럽히지 않는지 전체 후보 흐름으로 확인한다.
    #[test]
    fn candidate_preflight_rejects_before_squash_mutation() {
        let fixture = AcceptanceFixture::new("slice-accept-preflight-before-squash");
        fixture.write_message("updated");

        let error = fixture
            .integrate(|_, _| panic!("commit must not run"))
            .unwrap_err();

        assert!(error.contains("docs/src has no staged change"));
        assert_eq!(
            output(&fixture.repository.path, &["rev-parse", "HEAD"]),
            fixture.integration_head
        );
        assert!(status(&fixture.repository.path).is_empty());
    }

    // 사전검증, exact squash, staged 재검증, commit까지 같은 helper를 통과시켜 실제
    // integration commit의 canonical bytes가 후보 diff와 일치하는지 검증한다.
    #[test]
    fn candidate_integration_roundtrip_commits_the_exact_diff() {
        let fixture = AcceptanceFixture::new("slice-accept-roundtrip");
        fixture.write_message("none - Developer Docs responsibilities remain accurate");
        let expected = canonical_diff(
            &fixture.candidate,
            &fixture.integration_head,
            &fixture.candidate_head,
        )
        .unwrap();

        let accepted = fixture
            .integrate(|repository, message| {
                let status = git::command_in(repository, false)
                    .args(["commit", "--quiet", "--file"])
                    .arg(message)
                    .status()
                    .map_err(|error| format!("cannot start test commit: {error}"))?;
                status
                    .success()
                    .then_some(())
                    .ok_or_else(|| format!("test commit failed ({status})"))
            })
            .unwrap();

        assert_eq!(
            accepted,
            output(&fixture.repository.path, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            canonical_diff(&fixture.repository.path, &format!("{accepted}^"), &accepted).unwrap(),
            expected
        );
        assert!(status(&fixture.repository.path).is_empty());
    }
}
