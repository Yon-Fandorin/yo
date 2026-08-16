use super::{ImpactInput, changed_list, deferred_branch};

pub(crate) fn check(input: &ImpactInput) -> Result<(), String> {
    if deferred_branch(&input.branch) || input.merge_head.is_some() {
        return Ok(());
    }

    let changed = input
        .changed_paths
        .iter()
        .map(String::as_str)
        .filter(|path| {
            path.starts_with("crates/")
                || path.starts_with("shared/")
                || path.starts_with("tools/")
                || matches!(*path, "Cargo.toml" | "Cargo.lock" | ".cargo/config.toml")
        })
        .collect::<Vec<_>>();
    if changed.is_empty() {
        return Ok(());
    }

    let values = input
        .message
        .lines()
        .filter_map(|line| line.strip_prefix("Developer-Docs-Impact:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if values.len() != 1 {
        return Err(format!(
            "code changes require exactly one Developer-Docs-Impact trailer\n{}\n\
             use one of:\n  Developer-Docs-Impact: updated\n  \
             Developer-Docs-Impact: none - <why responsibilities and flows stay accurate>",
            changed_list(&changed)
        ));
    }

    match values[0] {
        "updated" => {
            if input
                .changed_paths
                .iter()
                .any(|path| path.starts_with("docs/src/"))
            {
                Ok(())
            } else {
                Err(
                    "Developer-Docs-Impact says updated, but docs/src has no staged change"
                        .to_owned(),
                )
            }
        },
        value
            if value
                .strip_prefix("none - ")
                .is_some_and(|reason| !reason.is_empty()) =>
        {
            Ok(())
        },
        value => Err(format!(
            "invalid Developer-Docs-Impact value: {value}\n\
             expected 'updated' or 'none - <reason>'"
        )),
    }
}

#[cfg(test)]
mod tests;
