use super::{ImpactInput, changed_list, deferred_branch};

pub(crate) fn check_commit(
    repository: &std::path::Path,
    commit: &str,
    branch: &str,
) -> Result<(), String> {
    let message = crate::git::output_in(
        repository,
        &["show", "--no-patch", "--format=%B", commit],
        false,
    )?;
    let changed_paths = crate::git::output_in(
        repository,
        &[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-only",
            "-r",
            commit,
        ],
        false,
    )?
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(str::to_owned)
    .collect();
    check(&ImpactInput {
        message,
        changed_paths,
        branch: branch.to_owned(),
        merge_head: None,
        repository: repository.to_path_buf(),
        inherit_git_environment: false,
    })
}

pub(crate) fn check(input: &ImpactInput) -> Result<(), String> {
    if deferred_branch(&input.branch) {
        return Ok(());
    }
    if input.branch.starts_with("wave/")
        && let Some(merge_head) = input.merge_head.as_deref()
        && crate::git::succeeds_in(
            &input.repository,
            &[
                "merge-base",
                "--is-ancestor",
                merge_head,
                "refs/heads/develop",
            ],
            input.inherit_git_environment,
        )?
    {
        return Ok(());
    }
    if input.branch.is_empty() {
        return Err(
            "Slice review impact cannot classify a detached or unresolved branch".to_owned(),
        );
    }

    let fresh_context_paths = input
        .changed_paths
        .iter()
        .map(String::as_str)
        .filter(|path| requires_fresh_context(path))
        .collect::<Vec<_>>();
    let code_quality_paths = input
        .changed_paths
        .iter()
        .map(String::as_str)
        .filter(|path| requires_code_quality(path))
        .collect::<Vec<_>>();
    let evidence = evidence(&input.message)?;

    if !fresh_context_paths.is_empty() && !evidence.has(Lens::FreshContext) {
        return Err(format!(
            "these changes require fresh-context review:\n{}\n{}",
            changed_list(&fresh_context_paths),
            with_usage("fresh-context review evidence is missing")
        ));
    }
    if !code_quality_paths.is_empty() && !evidence.has(Lens::CodeQuality) {
        return Err(format!(
            "these changes require code-quality review:\n{}\n{}",
            changed_list(&code_quality_paths),
            with_usage("code-quality review evidence is missing")
        ));
    }
    if input.branch.starts_with("wave/") && !evidence.has(Lens::Integration) {
        return Err(with_usage(
            "accepted Wave Slice commits require integration review evidence",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Lens {
    FreshContext,
    CodeQuality,
    Integration,
}

impl Lens {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "fresh-context" => Some(Self::FreshContext),
            "code-quality" => Some(Self::CodeQuality),
            "integration" => Some(Self::Integration),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::FreshContext => "fresh-context",
            Self::CodeQuality => "code-quality",
            Self::Integration => "integration",
        }
    }
}

pub(crate) struct CompletedReview {
    pub(crate) lens: Lens,
    pub(crate) reviewer: String,
}

pub(crate) struct Evidence {
    pub(crate) completed: Vec<CompletedReview>,
}

impl Evidence {
    fn has(&self, lens: Lens) -> bool {
        self.completed.iter().any(|review| review.lens == lens)
    }
}

pub(crate) fn evidence(message: &str) -> Result<Evidence, String> {
    let parsed = crate::git::interpret_trailers(message)?;
    let values = parsed
        .lines()
        .filter_map(|line| line.strip_prefix("Slice-Review:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Err(with_usage(
            "accepted commits must record Slice review disposition",
        ));
    }
    if values.len() == 1
        && values[0]
            .strip_prefix("none - ")
            .is_some_and(|reason| !reason.is_empty())
    {
        return Ok(Evidence {
            completed: Vec::new(),
        });
    }
    if values.iter().any(|value| value.starts_with("none - ")) {
        return Err(with_usage(
            "Slice-Review none cannot be combined with completed review lenses",
        ));
    }

    let mut completed = Vec::new();
    for value in values {
        let mut parts = value.split(" - ");
        let Some(lens) = parts.next().and_then(Lens::parse) else {
            return Err(with_usage("invalid Slice-Review trailer"));
        };
        if parts.next() != Some("completed") {
            return Err(with_usage("invalid Slice-Review trailer"));
        }
        let Some(reviewer) = parts.next().filter(|reviewer| valid_reviewer(reviewer)) else {
            return Err(with_usage("invalid Slice-Review trailer"));
        };
        if !matches!(parts.next(), Some("clear" | "resolved")) || parts.next().is_some() {
            return Err(with_usage("invalid Slice-Review trailer"));
        }
        if completed
            .iter()
            .any(|review: &CompletedReview| review.lens == lens)
        {
            return Err(with_usage(&format!(
                "{} review must be recorded exactly once",
                lens.label()
            )));
        }
        completed.push(CompletedReview {
            lens,
            reviewer: reviewer.to_owned(),
        });
    }
    Ok(Evidence { completed })
}

fn valid_reviewer(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters
            .all(|character| character.is_ascii_alphanumeric() || "._/+:-".contains(character))
}

fn requires_fresh_context(path: &str) -> bool {
    matches!(
        path,
        "AGENTS.md"
            | "README.md"
            | "CONTRIBUTING.md"
            | "hk.pkl"
            | "Cargo.toml"
            | "Cargo.lock"
            | ".cargo/config.toml"
            | "rust-toolchain"
            | "rust-toolchain.toml"
            | "rustfmt.toml"
            | ".gitignore"
            | "docs/book.toml"
    ) || [
        ".github/workflows/",
        "crates/",
        "shared/",
        "tools/",
        "docs-internal/design/",
        "methexis/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn requires_code_quality(path: &str) -> bool {
    ((path.starts_with("crates/") || path.starts_with("shared/") || path.starts_with("tools/"))
        && (path.ends_with(".rs") || path.ends_with(".sh")))
        || (path.starts_with("docs/theme/")
            && [".css", ".hbs", ".html", ".js"]
                .iter()
                .any(|suffix| path.ends_with(suffix)))
}

fn with_usage(message: &str) -> String {
    format!(
        "{message}\nrecord completed review evidence with one or more trailers:\n  \
         Slice-Review: fresh-context - completed - <reviewer-id> - <clear|resolved>\n  \
         Slice-Review: code-quality - completed - <reviewer-id> - <clear|resolved>\n  \
         Slice-Review: integration - completed - <reviewer-id> - <clear|resolved>\n\
         or, only when no lens is required:\n  \
         Slice-Review: none - <why no additional review lens applies>"
    )
}

#[cfg(test)]
mod tests;
