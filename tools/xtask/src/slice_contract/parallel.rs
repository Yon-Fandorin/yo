use std::{collections::BTreeSet, path::Path};

use super::{
    json,
    model::{self, PathRule},
    repository_root,
};
use crate::git;

pub(crate) fn check_parallel(
    repository: &Path,
    left_path: &Path,
    right_path: &Path,
) -> Result<(), String> {
    let left = model::load(left_path)?;
    let right = model::load(right_path)?;
    let repository = repository_root(repository)?;
    model::validate(&repository, &left)?;
    model::validate(&repository, &right)?;

    if left.slice == right.slice {
        return Err(format!("both contracts identify Slice `{}`", left.slice));
    }
    if left.base != right.base {
        return Err(format!(
            "Slices `{}` and `{}` do not share one base: {} != {}",
            left.slice, right.slice, left.base, right.base
        ));
    }
    if left.base_ref != right.base_ref {
        return Err(format!(
            "Slices `{}` and `{}` do not share one integration ref: {} != {}",
            left.slice, right.slice, left.base_ref, right.base_ref
        ));
    }

    let integration = git::output_in(
        &repository,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", left.base_ref),
        ],
        false,
    )?;
    let integration = integration.trim();
    if left.base != integration {
        return Err(format!(
            "parallel preflight base {} is stale; current {} is {integration}",
            left.base, left.base_ref
        ));
    }

    let left_rules = model::parse_rules(&left.allowed_write_set)?;
    let right_rules = model::parse_rules(&right.allowed_write_set)?;
    let overlaps = overlaps(&left_rules, &right_rules);
    if !overlaps.is_empty() {
        return Err(format!(
            "Slices `{}` and `{}` have overlapping write leases:\n{}",
            left.slice,
            right.slice,
            overlaps
                .iter()
                .map(|(left, right)| format!("- {} <> {}", left.display(), right.display()))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let shared_contracts = left
        .owned_contracts
        .iter()
        .filter(|contract| right.owned_contracts.contains(contract))
        .cloned()
        .collect::<Vec<_>>();
    if !shared_contracts.is_empty() {
        return Err(format!(
            "Slices `{}` and `{}` both own contracts: {}",
            left.slice,
            right.slice,
            shared_contracts.join(", ")
        ));
    }

    println!(
        "{{\"schema\":\"yo.slice-parallel-check/v1\",\"ok\":true,\"left\":{},\"right\":{},\"base\":{}}}",
        json(&left.slice)?,
        json(&right.slice)?,
        json(&left.base)?
    );
    Ok(())
}

pub(super) fn overlaps(left: &[PathRule], right: &[PathRule]) -> Vec<(PathRule, PathRule)> {
    let mut found = BTreeSet::new();
    for left_rule in left {
        for right_rule in right {
            if left_rule.contains(right_rule) || right_rule.contains(left_rule) {
                found.insert((left_rule.display(), right_rule.display()));
            }
        }
    }
    found
        .into_iter()
        .map(|(left, right)| {
            (
                PathRule::parse(&left).expect("displayed rule remains valid"),
                PathRule::parse(&right).expect("displayed rule remains valid"),
            )
        })
        .collect()
}
