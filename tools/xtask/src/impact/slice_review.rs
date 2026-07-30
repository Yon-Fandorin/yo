use super::{ImpactInput, changed_list, deferred_branch};

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
    let parsed = crate::git::interpret_trailers(&input.message)?;
    let values = parsed
        .lines()
        .filter_map(|line| line.strip_prefix("Slice-Review:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let recognized = values
        .iter()
        .filter_map(|value| Review::parse(value))
        .collect::<Vec<_>>();

    if values.is_empty() {
        return Err(with_usage(
            "accepted commits must record Slice review disposition",
        ));
    }
    if recognized.len() != values.len() {
        return Err(with_usage("invalid Slice-Review trailer"));
    }

    let none_count = count(&recognized, Review::None);
    if none_count > 0 && values.len() != 1 {
        return Err(with_usage(
            "Slice-Review none cannot be combined with completed review lenses",
        ));
    }
    for (review, label) in [
        (Review::FreshContext, "fresh-context"),
        (Review::CodeQuality, "code-quality"),
        (Review::Integration, "integration"),
    ] {
        if count(&recognized, review) > 1 {
            return Err(with_usage(&format!(
                "{label} review must be recorded exactly once"
            )));
        }
    }

    if !fresh_context_paths.is_empty() && count(&recognized, Review::FreshContext) == 0 {
        return Err(format!(
            "these changes require fresh-context review:\n{}\n{}",
            changed_list(&fresh_context_paths),
            with_usage("fresh-context review evidence is missing")
        ));
    }
    if !code_quality_paths.is_empty() && count(&recognized, Review::CodeQuality) == 0 {
        return Err(format!(
            "these changes require code-quality review:\n{}\n{}",
            changed_list(&code_quality_paths),
            with_usage("code-quality review evidence is missing")
        ));
    }
    if input.branch.starts_with("wave/") && count(&recognized, Review::Integration) == 0 {
        return Err(with_usage(
            "accepted Wave Slice commits require integration review evidence",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Review {
    FreshContext,
    CodeQuality,
    Integration,
    None,
}

impl Review {
    fn parse(value: &str) -> Option<Self> {
        if value
            .strip_prefix("none - ")
            .is_some_and(|reason| !reason.is_empty())
        {
            return Some(Self::None);
        }
        let mut parts = value.split(" - ");
        let lens = parts.next()?;
        if parts.next()? != "completed" || !valid_reviewer(parts.next()?) {
            return None;
        }
        if !matches!(parts.next()?, "clear" | "resolved") || parts.next().is_some() {
            return None;
        }
        match lens {
            "fresh-context" => Some(Self::FreshContext),
            "code-quality" => Some(Self::CodeQuality),
            "integration" => Some(Self::Integration),
            _ => None,
        }
    }
}

fn valid_reviewer(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters
            .all(|character| character.is_ascii_alphanumeric() || "._/+:-".contains(character))
}

fn count(reviews: &[Review], target: Review) -> usize {
    reviews.iter().filter(|review| **review == target).count()
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
        "tools/",
        "docs-internal/design/",
        "methexis/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn requires_code_quality(path: &str) -> bool {
    ((path.starts_with("crates/") || path.starts_with("tools/"))
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
