pub(crate) mod developer_docs;
pub(crate) mod slice_review;

use std::path::{Path, PathBuf};

pub(crate) struct ImpactInput {
    pub(crate) message: String,
    pub(crate) changed_paths: Vec<String>,
    pub(crate) branch: String,
    pub(crate) merge_head: Option<String>,
    pub(crate) repository: PathBuf,
    pub(crate) inherit_git_environment: bool,
}

impl ImpactInput {
    pub(crate) fn load(
        message_path: PathBuf,
        changed_paths_path: Option<PathBuf>,
        branch: Option<String>,
        head_fallback: bool,
    ) -> Result<Self, String> {
        let repository = std::env::current_dir()
            .map_err(|error| format!("cannot locate the repository: {error}"))?;
        Self::load_with_environment(
            &repository,
            message_path,
            changed_paths_path,
            branch,
            head_fallback,
            true,
        )
    }

    #[cfg(test)]
    pub(crate) fn load_from(
        repository: &Path,
        message_path: PathBuf,
        changed_paths_path: Option<PathBuf>,
        branch: Option<String>,
        head_fallback: bool,
    ) -> Result<Self, String> {
        Self::load_with_environment(
            repository,
            message_path,
            changed_paths_path,
            branch,
            head_fallback,
            false,
        )
    }

    fn load_with_environment(
        repository: &Path,
        message_path: PathBuf,
        changed_paths_path: Option<PathBuf>,
        branch: Option<String>,
        head_fallback: bool,
        inherit_git_environment: bool,
    ) -> Result<Self, String> {
        let message = crate::git::read(&message_path, "commit message")?;
        let branch = match branch {
            Some(branch) => branch,
            None => crate::git::optional_output_in(
                repository,
                &["symbolic-ref", "--quiet", "--short", "HEAD"],
                inherit_git_environment,
            )?
            .unwrap_or_default()
            .trim()
            .to_owned(),
        };
        let changed_paths = match changed_paths_path {
            Some(path) => lines(crate::git::read(&path, "changed paths")?),
            None => {
                let staged = crate::git::output_in(
                    repository,
                    &["diff", "--cached", "--name-only", "--diff-filter=ACDMR"],
                    inherit_git_environment,
                )?;
                if head_fallback
                    && staged.trim().is_empty()
                    && crate::git::succeeds_in(
                        repository,
                        &["rev-parse", "--quiet", "--verify", "HEAD"],
                        inherit_git_environment,
                    )?
                {
                    lines(crate::git::output_in(
                        repository,
                        &[
                            "diff-tree",
                            "--root",
                            "--no-commit-id",
                            "--name-only",
                            "-r",
                            "HEAD",
                        ],
                        inherit_git_environment,
                    )?)
                } else {
                    lines(staged)
                }
            },
        };
        let merge_head = crate::git::optional_output_in(
            repository,
            &["rev-parse", "--quiet", "--verify", "MERGE_HEAD"],
            inherit_git_environment,
        )?
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
        Ok(Self {
            message,
            changed_paths,
            branch,
            merge_head,
            repository: repository.to_path_buf(),
            inherit_git_environment,
        })
    }
}

fn lines(value: String) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn deferred_branch(branch: &str) -> bool {
    ["slice/", "task/", "spike/"]
        .iter()
        .any(|prefix| branch.starts_with(prefix))
}

pub(crate) fn changed_list(paths: &[&str]) -> String {
    paths
        .iter()
        .map(|path| format!("  changed: {path}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;
