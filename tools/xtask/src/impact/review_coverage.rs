use std::{collections::BTreeMap, path::Path};

use super::{ImpactInput, deferred_branch, slice_review};
use crate::{git, review_protocol::digest};

mod commit;
mod model;

use model::Coverage;

const CUTOVER_PARENT: &str = "edf376fd33dc10e8fa3e02ca0e4543025249838a";

pub(crate) fn check(input: &ImpactInput) -> Result<(), String> {
    if deferred_branch(&input.branch) {
        return Ok(());
    }
    check_with_cutover(input, CUTOVER_PARENT)
}

pub(crate) fn check_commit(repository: &Path, commit: &str, branch: &str) -> Result<(), String> {
    check_commit_with_cutover(repository, commit, branch, CUTOVER_PARENT)
}

pub(crate) fn check_prepare_commit_message(
    repository: &Path,
    source: Option<&str>,
    commit: Option<&str>,
) -> Result<(), String> {
    check_prepare_commit_message_with_cutover(repository, source, commit, CUTOVER_PARENT)
}

pub(crate) fn create_accepted_commit(repository: &Path, message: &Path) -> Result<(), String> {
    commit::create(repository, message)
}

pub(crate) fn copy_accepted_commit_message(target: &Path) -> Result<(), String> {
    commit::copy_message(target)
}

pub(crate) fn reviewer_for_route(value: &str, lens: slice_review::Lens) -> Result<String, String> {
    let reviewer =
        model::Reviewer::parse(value).ok_or_else(|| format!("invalid review route `{value}`"))?;
    if lens != slice_review::Lens::CodeQuality && !reviewer.is_high_or_human() {
        return Err(format!(
            "{} review requires a model-high or human route",
            lens.label()
        ));
    }
    Ok(reviewer.compact_id())
}

pub(crate) fn human_reviewer_for_route(value: &str) -> Option<String> {
    let reviewer = model::Reviewer::parse(value)?;
    reviewer.is_human().then(|| reviewer.compact_id())
}

fn check_with_cutover(input: &ImpactInput, cutover: &str) -> Result<(), String> {
    if !git::succeeds_in(
        &input.repository,
        &["merge-base", "--is-ancestor", cutover, "HEAD"],
        input.inherit_git_environment,
    )? {
        return Ok(());
    }
    let bytes = current_review_diff(input)?;
    validate(&input.message, &digest(&bytes))
}

fn check_commit_with_cutover(
    repository: &Path,
    commit: &str,
    _branch: &str,
    cutover: &str,
) -> Result<(), String> {
    if commit == cutover
        || !git::succeeds_in(
            repository,
            &["merge-base", "--is-ancestor", cutover, commit],
            false,
        )?
    {
        return Ok(());
    }
    let message = git::output_in(
        repository,
        &["show", "--no-patch", "--format=%B", commit],
        false,
    )?;
    let parent = format!("{commit}^");
    let bytes = canonical_diff(repository, &parent, commit, false)?;
    validate(&message, &digest(&bytes))
}

fn check_prepare_commit_message_with_cutover(
    repository: &Path,
    source: Option<&str>,
    commit: Option<&str>,
    cutover: &str,
) -> Result<(), String> {
    let branch = git::optional_output_in(
        repository,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        false,
    )?
    .unwrap_or_default();
    if deferred_branch(branch.trim())
        || !git::succeeds_in(
            repository,
            &["merge-base", "--is-ancestor", cutover, "HEAD"],
            false,
        )?
    {
        return Ok(());
    }

    match (source, commit) {
        (None, None) | (Some("merge" | "squash"), None) => Ok(()),
        (Some("message" | "template" | "commit"), _) => Err(
            "accepted integration commits after the review-coverage cutover reject -m, -F, \
             -t, -c, -C, and --amend because Git reports ambiguous combinations only as a \
             message source; use `cargo xtask slice commit <commit-message-file>` for a \
             prepared message, or plain `git commit` for a human-edited new commit"
                .to_owned(),
        ),
        _ => Err(
            "unexpected prepare-commit-msg source/commit arguments; refusing an ambiguous \
             accepted review surface"
                .to_owned(),
        ),
    }
}

fn current_review_diff(input: &ImpactInput) -> Result<Vec<u8>, String> {
    let staged = git::output_bytes_in(
        &input.repository,
        &[
            "diff",
            "--cached",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-renames",
            "HEAD",
            "--",
        ],
        input.inherit_git_environment,
    )?;
    if !staged.is_empty() {
        return Ok(staged);
    }
    canonical_diff(
        &input.repository,
        "HEAD^",
        "HEAD",
        input.inherit_git_environment,
    )
}

fn canonical_diff(
    repository: &Path,
    base: &str,
    candidate: &str,
    inherit_git_environment: bool,
) -> Result<Vec<u8>, String> {
    git::output_bytes_in(
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
        inherit_git_environment,
    )
}

fn validate(message: &str, expected_hash: &str) -> Result<(), String> {
    let evidence = match slice_review::evidence(message) {
        Ok(evidence) => evidence,
        Err(_) => return Ok(()),
    };
    let parsed = git::interpret_trailers(message)?;
    let values = parsed
        .lines()
        .filter_map(|line| line.strip_prefix("Review-Coverage:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if evidence.completed.is_empty() {
        return if values.is_empty() {
            Ok(())
        } else {
            Err("Review-Coverage cannot accompany Slice-Review none".to_owned())
        };
    }
    if values.is_empty() {
        return Err(usage(
            "accepted commits must bind every completed review lens to the exact diff",
            expected_hash,
            &evidence,
        ));
    }

    let mut coverage = BTreeMap::new();
    for value in values {
        let entry = Coverage::parse(value)
            .ok_or_else(|| usage("invalid Review-Coverage trailer", expected_hash, &evidence))?;
        if coverage.insert(entry.lens, entry).is_some() {
            return Err(usage(
                "each review lens must have exactly one Review-Coverage trailer",
                expected_hash,
                &evidence,
            ));
        }
    }

    if coverage.len() != evidence.completed.len() {
        return Err(usage(
            "Review-Coverage lenses must exactly match completed Slice-Review lenses",
            expected_hash,
            &evidence,
        ));
    }
    for review in &evidence.completed {
        let Some(entry) = coverage.get(&review.lens) else {
            return Err(usage(
                "Review-Coverage lenses must exactly match completed Slice-Review lenses",
                expected_hash,
                &evidence,
            ));
        };
        if entry.reviewer.compact_id() != review.reviewer {
            return Err(usage(
                "Review-Coverage reviewer must identify the matching Slice-Review reviewer",
                expected_hash,
                &evidence,
            ));
        }
        if review.lens != slice_review::Lens::CodeQuality && !entry.reviewer.is_high_or_human() {
            return Err(usage(
                "fresh-context and integration coverage require model-high or human exact review",
                expected_hash,
                &evidence,
            ));
        }
        if entry.diff_hash != expected_hash {
            return Err(usage(
                "Review-Coverage diff hash does not match the accepted review surface",
                expected_hash,
                &evidence,
            ));
        }
    }
    Ok(())
}

fn usage(message: &str, expected_hash: &str, evidence: &slice_review::Evidence) -> String {
    let examples = evidence
        .completed
        .iter()
        .map(|review| {
            let reviewer = if review.lens == slice_review::Lens::CodeQuality {
                "<model/provider/model/session|model-high/provider/model/session|delegated/host/session|delegated-high/host/session|human/name>"
            } else {
                "<model-high/provider/model/session|delegated-high/host/session|human/name>"
            };
            format!(
                "  Review-Coverage: {} - exact - {reviewer} - {expected_hash}",
                review.lens.label()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{message}\nrecord exact accepted-review coverage for the completed lenses with:\n\
         {examples}"
    )
}

#[cfg(test)]
#[path = "review_coverage/tests.rs"]
mod tests;
