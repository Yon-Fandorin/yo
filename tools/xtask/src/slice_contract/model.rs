use std::{collections::BTreeSet, path::Path};

use serde::Deserialize;

use super::{git_output, git_succeeds};

pub(super) const SCHEMA: &str = "yo.slice-contract/v1";
pub(super) const BINDING_FILE: &str = "yo-slice-contract";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SliceContract {
    pub(crate) schema: String,
    pub(crate) slice: String,
    pub(crate) base: String,
    pub(crate) base_ref: String,
    pub(crate) owned_contracts: Vec<String>,
    #[serde(default)]
    pub(crate) dependencies: Vec<String>,
    pub(crate) allowed_write_set: Vec<String>,
    pub(crate) focused_checks: Vec<String>,
    pub(crate) slice_close_checks: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PathRule {
    Exact(String),
    Tree(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundSlice {
    pub(crate) slice: String,
    pub(crate) base: String,
    pub(crate) base_ref: String,
    pub(crate) binding_path: std::path::PathBuf,
    pub(crate) contract_path: std::path::PathBuf,
    pub(crate) contract_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnsuredBinding {
    pub(crate) slice: String,
    pub(crate) contract_path: std::path::PathBuf,
    pub(crate) binding_path: std::path::PathBuf,
    pub(crate) created: bool,
}

pub(crate) fn load(path: &Path) -> Result<SliceContract, String> {
    read_contract(path).map(|(contract, _)| contract)
}

pub(super) fn read_contract(path: &Path) -> Result<(SliceContract, Vec<u8>), String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read Slice contract {}: {error}", path.display()))?;
    let contract = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Slice contract {}: {error}", path.display()))?;
    Ok((contract, bytes))
}

pub(crate) fn validate(repository: &Path, contract: &SliceContract) -> Result<(), String> {
    validate_with(repository, contract, false)
}

pub(super) fn validate_with(
    repository: &Path,
    contract: &SliceContract,
    trusted: bool,
) -> Result<(), String> {
    if contract.schema != SCHEMA {
        return Err(format!(
            "unsupported Slice contract schema `{}`; expected `{SCHEMA}`",
            contract.schema
        ));
    }
    if contract.slice.trim().is_empty() || contract.slice != contract.slice.trim() {
        return Err("Slice name must be non-empty and have no surrounding whitespace".to_owned());
    }
    if contract.owned_contracts.is_empty() {
        return Err(format!(
            "Slice `{}` must declare at least one owned contract",
            contract.slice
        ));
    }
    ensure_distinct_non_empty("owned contract", &contract.owned_contracts)?;
    ensure_distinct_non_empty("dependency", &contract.dependencies)?;
    ensure_distinct_non_empty("focused check", &contract.focused_checks)?;
    ensure_distinct_non_empty("Slice-close check", &contract.slice_close_checks)?;
    if contract.allowed_write_set.is_empty() {
        return Err(format!(
            "Slice `{}` must declare a non-empty allowed write-set",
            contract.slice
        ));
    }
    if contract.focused_checks.is_empty() || contract.slice_close_checks.is_empty() {
        return Err(format!(
            "Slice `{}` must declare focused and Slice-close checks",
            contract.slice
        ));
    }
    if !matches!(contract.base.len(), 40 | 64)
        || !contract.base.bytes().all(|byte| byte.is_ascii_hexdigit())
        || contract.base.bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(format!(
            "Slice `{}` base must be a full lowercase hexadecimal commit ID",
            contract.slice
        ));
    }
    if contract.base_ref != "refs/heads/develop"
        && !contract.base_ref.starts_with("refs/heads/wave/")
    {
        return Err(format!(
            "Slice `{}` base_ref must identify develop or a Wave integration branch",
            contract.slice
        ));
    }
    let valid_ref = git_succeeds(
        repository,
        &["check-ref-format", &contract.base_ref],
        trusted,
    )?;
    if !valid_ref {
        return Err(format!(
            "Slice `{}` has invalid base_ref `{}`",
            contract.slice, contract.base_ref
        ));
    }

    parse_rules(&contract.allowed_write_set)?;

    let base_reference = format!("{}^{{commit}}", contract.base);
    let arguments = ["rev-parse", "--verify", base_reference.as_str()];
    let resolved = git_output(repository, &arguments, trusted)?;
    if resolved.trim() != contract.base {
        return Err(format!(
            "Slice `{}` base must be a full canonical commit ID; `{}` resolves to {}",
            contract.slice,
            contract.base,
            resolved.trim()
        ));
    }
    let base_is_ancestor = git_succeeds(
        repository,
        &["merge-base", "--is-ancestor", &contract.base, "HEAD"],
        trusted,
    )?;
    if !base_is_ancestor {
        return Err(format!(
            "Slice `{}` base {} is not an ancestor of HEAD",
            contract.slice, contract.base
        ));
    }
    let base_belongs_to_integration = git_succeeds(
        repository,
        &[
            "merge-base",
            "--is-ancestor",
            &contract.base,
            &contract.base_ref,
        ],
        trusted,
    )?;
    if !base_belongs_to_integration {
        return Err(format!(
            "Slice `{}` base {} does not belong to integration history {}",
            contract.slice, contract.base, contract.base_ref
        ));
    }
    Ok(())
}

pub(super) fn validate_slice_branch(
    repository: &Path,
    contract: &SliceContract,
) -> Result<(), String> {
    validate_slice_branch_with(repository, contract, false)
}

pub(super) fn validate_slice_branch_with(
    repository: &Path,
    contract: &SliceContract,
    trusted: bool,
) -> Result<(), String> {
    let arguments = ["symbolic-ref", "--quiet", "--short", "HEAD"];
    let branch = git_output(repository, &arguments, trusted).map_err(|_| {
        format!(
            "Slice contract `{}` requires a named Slice or Task branch; HEAD is detached",
            contract.slice
        )
    })?;
    let branch = branch.trim();
    let segments = branch.split('/').collect::<Vec<_>>();
    let branch_base_ref = match segments.as_slice() {
        ["slice", "direct", slice] if *slice == contract.slice => "refs/heads/develop".to_owned(),
        ["task", "direct", slice, task] if *slice == contract.slice && !task.is_empty() => {
            "refs/heads/develop".to_owned()
        },
        ["slice", wave, slice]
            if !wave.is_empty() && *wave != "direct" && *slice == contract.slice =>
        {
            format!("refs/heads/wave/{wave}")
        },
        ["task", wave, slice, task]
            if !wave.is_empty()
                && *wave != "direct"
                && *slice == contract.slice
                && !task.is_empty() =>
        {
            format!("refs/heads/wave/{wave}")
        },
        _ => {
            return Err(format!(
                "Slice contract `{}` does not match current Slice or Task branch `{branch}`",
                contract.slice
            ));
        },
    };
    if branch_base_ref != contract.base_ref {
        return Err(format!(
            "branch `{branch}` belongs to `{branch_base_ref}`, but Slice contract `{}` declares `{}`",
            contract.slice, contract.base_ref
        ));
    }
    Ok(())
}

pub(super) fn ensure_distinct_non_empty(label: &str, values: &[String]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() || value != value.trim() {
            return Err(format!(
                "{label} must be non-empty and have no surrounding whitespace"
            ));
        }
        if !seen.insert(value) {
            return Err(format!("duplicate {label} `{value}`"));
        }
    }
    Ok(())
}

pub(super) fn parse_rules(raw: &[String]) -> Result<Vec<PathRule>, String> {
    raw.iter().map(|value| PathRule::parse(value)).collect()
}

impl PathRule {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        if value.is_empty()
            || value.starts_with('/')
            || value.contains('\\')
            || value.split('/').any(|part| matches!(part, "" | "." | ".."))
        {
            return Err(format!("invalid repository-relative write rule `{value}`"));
        }
        if let Some(prefix) = value.strip_suffix("/**") {
            if prefix.contains(['*', '?', '[', ']']) {
                return Err(format!("unsupported write rule `{value}`"));
            }
            return Ok(Self::Tree(prefix.to_owned()));
        }
        if value.contains(['*', '?', '[', ']']) {
            return Err(format!("unsupported write rule `{value}`"));
        }
        Ok(Self::Exact(value.to_owned()))
    }

    pub(super) fn matches(&self, path: &str) -> bool {
        match self {
            Self::Exact(exact) => path == exact,
            Self::Tree(tree) => path == tree || path.starts_with(&format!("{tree}/")),
        }
    }

    pub(super) fn contains(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Exact(left), Self::Exact(right)) => left == right,
            (Self::Exact(_), Self::Tree(_)) => false,
            (Self::Tree(left), Self::Exact(right)) => {
                right == left || right.starts_with(&format!("{left}/"))
            },
            (Self::Tree(left), Self::Tree(right)) => {
                right == left || right.starts_with(&format!("{left}/"))
            },
        }
    }

    pub(super) fn display(&self) -> String {
        match self {
            Self::Exact(path) => path.clone(),
            Self::Tree(path) => format!("{path}/**"),
        }
    }
}
