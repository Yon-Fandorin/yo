use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

#[cfg(test)]
mod tests;

use super::current_repository;
use crate::{impact, impact::ImpactInput, slice_contract, test_explanations, validation_stage};

pub(super) fn run(
    check: &OsStr,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let check = check.to_string_lossy();
    match check.as_ref() {
        "test-explanations" => run_test_explanations_check(arguments),
        "slice-scope" => run_slice_scope_check(arguments),
        "slice-parallel" => run_slice_parallel_check(arguments),
        "wave-assembly" => run_wave_assembly_check(arguments),
        "methexis-check-for-stage" => run_methexis_check_for_stage(arguments),
        "review-coverage-operation" => run_review_coverage_operation_check(arguments),
        "change-preflight"
        | "commit-preflight"
        | "developer-docs-impact"
        | "slice-review-impact" => run_impact_check(arguments, check.as_ref()),
        _ => Err(usage(check.as_ref())),
    }
}

fn run_test_explanations_check(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    if arguments.next().is_some() {
        return Err(usage("test-explanations"));
    }
    let repository = current_repository()?;
    test_explanations::check(&repository)
}

fn run_slice_scope_check(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let contract = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err(usage("slice-scope"));
    }
    let repository = current_repository()?;
    match contract {
        Some(contract) => slice_contract::check_scope(&repository, &contract),
        None => slice_contract::check_bound_scope(&repository),
    }
}

fn run_slice_parallel_check(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let left = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("slice-parallel"))?;
    let right = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("slice-parallel"))?;
    if arguments.next().is_some() {
        return Err(usage("slice-parallel"));
    }
    let repository = current_repository()?;
    slice_contract::check_parallel(&repository, &left, &right)
}

fn run_wave_assembly_check(arguments: &mut impl Iterator<Item = OsString>) -> Result<(), String> {
    let boundary = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("wave-assembly"))?;
    let components = arguments.map(PathBuf::from).collect::<Vec<_>>();
    if components.is_empty() {
        return Err(usage("wave-assembly"));
    }
    let repository = current_repository()?;
    slice_contract::check_wave_assembly(&repository, &boundary, &components)
}

fn run_methexis_check_for_stage(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    if arguments.next().is_some() {
        return Err(usage("methexis-check-for-stage"));
    }
    let repository = current_repository()?;
    validation_stage::run_methexis_check(&repository)
}

fn run_review_coverage_operation_check(
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), String> {
    let _message = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage("review-coverage-operation"))?;
    let source = arguments
        .next()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "prepare-commit-msg source must be valid UTF-8".to_owned())
        })
        .transpose()?;
    let commit = arguments
        .next()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "prepare-commit-msg commit must be valid UTF-8".to_owned())
        })
        .transpose()?;
    if arguments.next().is_some() {
        return Err(usage("review-coverage-operation"));
    }
    let repository = current_repository()?;
    impact::review_coverage::check_prepare_commit_message(
        &repository,
        source.as_deref(),
        commit.as_deref(),
    )
}

fn run_impact_check(
    arguments: &mut impl Iterator<Item = OsString>,
    check: &str,
) -> Result<(), String> {
    let head_fallback = matches!(
        check,
        "change-preflight" | "commit-preflight" | "slice-review-impact"
    );
    let message = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| usage(check))?;
    let changed_paths = arguments.next().map(PathBuf::from);
    let branch = arguments
        .next()
        .map(|value| value.to_string_lossy().into_owned());
    if arguments.next().is_some() {
        return Err(usage(check));
    }
    let input = ImpactInput::load(message, changed_paths, branch, head_fallback)?;
    match check {
        "change-preflight" => impact::change::check(&input),
        "commit-preflight" => impact::preflight::check(&input),
        "developer-docs-impact" => impact::developer_docs::check(&input),
        "slice-review-impact" => impact::slice_review::check(&input),
        _ => unreachable!("the check name was validated before loading input"),
    }
}

fn usage(check: &str) -> String {
    match check {
        "test-explanations" | "methexis-check-for-stage" => {
            return format!("usage: cargo xtask check {check}");
        },
        "slice-scope" => {
            return "usage: cargo xtask check slice-scope [slice-contract.json]".to_owned();
        },
        "slice-parallel" => {
            return "usage: cargo xtask check slice-parallel <left.json> <right.json>".to_owned();
        },
        "wave-assembly" => {
            return "usage: cargo xtask check wave-assembly <boundary.json> <component.json>..."
                .to_owned();
        },
        "review-coverage-operation" => {
            return "usage: cargo xtask check review-coverage-operation \
                    <commit-message-file> [source] [commit]"
                .to_owned();
        },
        _ => {},
    }
    format!(
        "usage: cargo xtask check {} <commit-message-file> [changed-paths-file] [branch]",
        check
    )
}
