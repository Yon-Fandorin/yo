use std::{collections::BTreeMap, path::Path};

use super::{
    model::{Request, ResultDocument, ReviewResult},
    revalidate::{REQUEST_LIMIT, captured},
};
use crate::{bounded_file, review_protocol, validation_summary};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadyGate {
    pub(crate) slice: String,
    pub(crate) candidate_commit: String,
    pub(crate) diff_hash: String,
    pub(crate) validation: Vec<ReadyValidation>,
    pub(crate) review_count: usize,
    pub(crate) known_unverified_environments: Vec<String>,
    pub(crate) commit_trailers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadyValidation {
    pub(crate) name: String,
    pub(crate) argv: Vec<String>,
    pub(crate) status: String,
    pub(crate) reused: bool,
    pub(crate) current_reusable_context: bool,
}
pub(super) fn ready(
    repository: &Path,
    request_path: &Path,
    evaluate: &dyn Fn(&Path, &Path) -> Result<ResultDocument, String>,
) -> Result<ReadyGate, String> {
    let result = evaluate(repository, request_path)?;
    if result.next_action != "integrate" {
        return Err(format!(
            "post-gate preparation requires next_action `integrate`, found `{}`",
            result.next_action
        ));
    }

    let request_bytes =
        bounded_file::read_regular(request_path, REQUEST_LIMIT, "Slice gate request")?;
    if review_protocol::digest(&request_bytes) != result.request_hash {
        return Err("Slice gate request changed after ready evaluation".to_owned());
    }
    let request: Request = serde_json::from_slice(&request_bytes).map_err(|error| {
        format!(
            "invalid Slice gate request {} after ready evaluation: {error}",
            request_path.display()
        )
    })?;
    let mut commands = request
        .validation_evidence
        .into_iter()
        .map(|entry| {
            let bytes = captured(
                repository,
                &entry.result_path,
                &entry.result_hash,
                "validation result",
            )?;
            let current_context =
                validation_summary::current_reusable_context(&bytes).map_err(|error| {
                    format!(
                        "cannot revalidate current context for `{}`: {error}",
                        entry.name
                    )
                })?;
            Ok((entry.name, (entry.argv, current_context)))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let validation = result
        .validation
        .into_iter()
        .map(|entry| {
            let (argv, current_reusable_context) =
                commands.remove(&entry.name).ok_or_else(|| {
                    format!(
                        "ready gate validation `{}` lost its source command",
                        entry.name
                    )
                })?;
            Ok(ReadyValidation {
                name: entry.name,
                argv,
                status: entry.status,
                reused: entry.reused,
                current_reusable_context,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !commands.is_empty() {
        return Err("ready gate result omitted a source validation command".to_owned());
    }

    Ok(ReadyGate {
        slice: result.slice,
        candidate_commit: result.candidate_commit,
        diff_hash: result.diff_hash,
        validation,
        review_count: result.review.len(),
        known_unverified_environments: result.known_unverified_environments,
        commit_trailers: result.commit_trailers,
    })
}
pub(super) fn trailers(
    review: &[ReviewResult],
    diff_hash: &str,
    no_lens_required: bool,
) -> Vec<String> {
    if no_lens_required {
        return vec![
            "Slice-Review: none - no path-based or planner-added review lens applies".to_owned(),
        ];
    }
    let mut trailers = Vec::new();
    for entry in review {
        trailers.push(format!(
            "Slice-Review: {} - completed - {} - {}",
            entry.lens, entry.reviewer, entry.verdict
        ));
    }
    for entry in review {
        trailers.push(format!(
            "Review-Coverage: {} - exact - {} - {diff_hash}",
            entry.lens, entry.route
        ));
    }
    trailers
}
