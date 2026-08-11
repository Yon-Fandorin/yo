use super::{ImpactInput, developer_docs, review_coverage, slice_review};

pub(crate) fn check(input: &ImpactInput) -> Result<(), String> {
    combine(
        slice_review::check(input),
        review_coverage::check(input),
        developer_docs::check(input),
    )
}

fn combine(
    review: Result<(), String>,
    coverage: Result<(), String>,
    developer_docs: Result<(), String>,
) -> Result<(), String> {
    let mut diagnostics = Vec::new();
    if let Err(error) = review {
        diagnostics.push(format!("Slice review impact:\n{error}"));
    }
    if let Err(error) = coverage {
        diagnostics.push(format!("Accepted review coverage:\n{error}"));
    }
    if let Err(error) = developer_docs {
        diagnostics.push(format!("Developer Docs impact:\n{error}"));
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "commit preflight failed:\n\n{}",
            diagnostics.join("\n\n")
        ))
    }
}

#[cfg(test)]
#[path = "preflight/tests.rs"]
mod tests;
