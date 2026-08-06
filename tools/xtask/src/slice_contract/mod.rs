use std::{
    collections::BTreeSet,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::git;

const SCHEMA: &str = "yo.slice-contract/v1";
const BINDING_FILE: &str = "yo-slice-contract";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SliceContract {
    schema: String,
    slice: String,
    base: String,
    base_ref: String,
    owned_contracts: Vec<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    allowed_write_set: Vec<String>,
    focused_checks: Vec<String>,
    slice_close_checks: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PathRule {
    Exact(String),
    Tree(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundSlice {
    pub(crate) slice: String,
    pub(crate) base: String,
    pub(crate) base_ref: String,
    pub(crate) binding_path: PathBuf,
    pub(crate) contract_path: PathBuf,
    pub(crate) contract_id: String,
}

pub(crate) fn check_scope(repository: &Path, contract_path: &Path) -> Result<(), String> {
    let index_file = selected_index(repository);
    check_scope_with_index(repository, contract_path, index_file.as_deref())
}

fn selected_index(repository: &Path) -> Option<PathBuf> {
    std::env::var_os("GIT_INDEX_FILE").map(|value| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            repository.join(path)
        }
    })
}

pub(crate) fn check_bound_scope(repository: &Path) -> Result<(), String> {
    let index_file = selected_index(repository);
    check_bound_scope_with_index(repository, index_file.as_deref())
}

pub(crate) fn bound_slice(repository: &Path) -> Result<BoundSlice, String> {
    let repository = repository_root(repository)?;
    let binding_path = binding_path(&repository)?;
    let contract_path = bound_contract_path(&repository)?;
    let (contract, bytes) = read_contract(&contract_path)?;
    validate(&repository, &contract)?;
    validate_slice_branch(&repository, &contract)?;
    Ok(BoundSlice {
        slice: contract.slice,
        base: contract.base,
        base_ref: contract.base_ref,
        binding_path,
        contract_path,
        contract_id: format!(
            "sha256:{}",
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ),
    })
}

fn check_bound_scope_with_index(
    repository: &Path,
    index_file: Option<&Path>,
) -> Result<(), String> {
    let contract_path = bound_contract_path(repository)?;
    let contract = load(&contract_path)?;
    let repository = repository_root(repository)?;
    validate_slice_branch(&repository, &contract)?;
    check_scope_with_index(&repository, &contract_path, index_file)
}

pub(crate) fn bind(repository: &Path, contract_path: &Path) -> Result<(), String> {
    let contract_path = std::fs::canonicalize(contract_path).map_err(|error| {
        format!(
            "cannot resolve Slice contract {}: {error}",
            contract_path.display()
        )
    })?;
    let contract = load(&contract_path)?;
    let repository = repository_root(repository)?;
    validate(&repository, &contract)?;
    validate_slice_branch(&repository, &contract)?;

    let binding = binding_path(&repository)?;
    std::fs::write(&binding, format!("{}\n", contract_path.display())).map_err(|error| {
        format!(
            "cannot bind Slice contract at {}: {error}",
            binding.display()
        )
    })?;

    println!(
        "{{\"schema\":\"yo.slice-contract-binding/v1\",\"ok\":true,\"slice\":{},\"contract_path\":{}}}",
        json(&contract.slice)?,
        json(&contract_path)?
    );
    Ok(())
}

fn check_scope_with_index(
    repository: &Path,
    contract_path: &Path,
    index_file: Option<&Path>,
) -> Result<(), String> {
    let contract = load(contract_path)?;
    let repository = repository_root(repository)?;
    validate(&repository, &contract)?;
    reject_hidden_index_paths(&repository, index_file.map(Path::as_os_str))?;

    let changed = changed_paths(&repository, &contract.base, index_file.map(Path::as_os_str))?;
    let rules = parse_rules(&contract.allowed_write_set)?;
    let outside = changed
        .iter()
        .filter(|path| !rules.iter().any(|rule| rule.matches(path)))
        .cloned()
        .collect::<Vec<_>>();
    if !outside.is_empty() {
        return Err(format!(
            "Slice `{}` changed paths outside its allowed write-set:\n{}",
            contract.slice,
            outside
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    println!(
        "{{\"schema\":\"yo.slice-scope-check/v1\",\"ok\":true,\"slice\":{},\"contract_path\":{},\"base\":{},\"changed_paths\":{}}}",
        json(&contract.slice)?,
        json(&contract_path)?,
        json(&contract.base)?,
        json(&changed)?
    );
    Ok(())
}

pub(crate) fn check_parallel(
    repository: &Path,
    left_path: &Path,
    right_path: &Path,
) -> Result<(), String> {
    let left = load(left_path)?;
    let right = load(right_path)?;
    let repository = repository_root(repository)?;
    validate(&repository, &left)?;
    validate(&repository, &right)?;

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

    let left_rules = parse_rules(&left.allowed_write_set)?;
    let right_rules = parse_rules(&right.allowed_write_set)?;
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

fn load(path: &Path) -> Result<SliceContract, String> {
    read_contract(path).map(|(contract, _)| contract)
}

fn read_contract(path: &Path) -> Result<(SliceContract, Vec<u8>), String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read Slice contract {}: {error}", path.display()))?;
    let contract = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Slice contract {}: {error}", path.display()))?;
    Ok((contract, bytes))
}

fn repository_root(directory: &Path) -> Result<PathBuf, String> {
    let output = git::output_in(directory, &["rev-parse", "--show-toplevel"], false)?;
    let root = output.trim();
    if root.is_empty() {
        return Err("git rev-parse --show-toplevel returned an empty path".to_owned());
    }
    Ok(PathBuf::from(root))
}

fn binding_path(repository: &Path) -> Result<PathBuf, String> {
    let output = git::output_in(
        repository,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            BINDING_FILE,
        ],
        false,
    )?;
    let path = output.trim();
    if path.is_empty() {
        return Err("git rev-parse --git-path returned an empty binding path".to_owned());
    }
    Ok(PathBuf::from(path))
}

fn bound_contract_path(repository: &Path) -> Result<PathBuf, String> {
    let repository = repository_root(repository)?;
    let binding = binding_path(&repository)?;
    let value = std::fs::read_to_string(&binding).map_err(|error| {
        format!(
            "this worktree has no readable Slice contract binding at {}: {error}\n\
             ask the planner to run `cargo xtask slice-contract bind <slice-contract.json>`",
            binding.display()
        )
    })?;
    let path = value.trim();
    if path.is_empty() || value.lines().count() != 1 {
        return Err(format!(
            "Slice contract binding {} must contain exactly one non-empty path",
            binding.display()
        ));
    }
    Ok(PathBuf::from(path))
}

fn validate_slice_branch(repository: &Path, contract: &SliceContract) -> Result<(), String> {
    let branch = git::output_in(
        repository,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        false,
    )
    .map_err(|_| {
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

fn validate(repository: &Path, contract: &SliceContract) -> Result<(), String> {
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
    if !git::succeeds_in(repository, &["check-ref-format", &contract.base_ref], false)? {
        return Err(format!(
            "Slice `{}` has invalid base_ref `{}`",
            contract.slice, contract.base_ref
        ));
    }

    parse_rules(&contract.allowed_write_set)?;

    let resolved = git::output_in(
        repository,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", contract.base),
        ],
        false,
    )?;
    if resolved.trim() != contract.base {
        return Err(format!(
            "Slice `{}` base must be a full canonical commit ID; `{}` resolves to {}",
            contract.slice,
            contract.base,
            resolved.trim()
        ));
    }
    if !git::succeeds_in(
        repository,
        &["merge-base", "--is-ancestor", &contract.base, "HEAD"],
        false,
    )? {
        return Err(format!(
            "Slice `{}` base {} is not an ancestor of HEAD",
            contract.slice, contract.base
        ));
    }
    if !git::succeeds_in(
        repository,
        &[
            "merge-base",
            "--is-ancestor",
            &contract.base,
            &contract.base_ref,
        ],
        false,
    )? {
        return Err(format!(
            "Slice `{}` base {} does not belong to integration history {}",
            contract.slice, contract.base, contract.base_ref
        ));
    }
    Ok(())
}

fn ensure_distinct_non_empty(label: &str, values: &[String]) -> Result<(), String> {
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

fn changed_paths(
    repository: &Path,
    base: &str,
    index_file: Option<&OsStr>,
) -> Result<Vec<String>, String> {
    let mut paths = diff_paths(git::output_bytes_in_with_index(
        repository,
        &[
            "diff",
            "--name-status",
            "-z",
            "--no-renames",
            base,
            "HEAD",
            "--",
        ],
        index_file,
    )?)?;
    paths.extend(diff_paths(git::output_bytes_in_with_index(
        repository,
        &[
            "diff",
            "--cached",
            "--name-status",
            "-z",
            "--no-renames",
            "HEAD",
            "--",
        ],
        index_file,
    )?)?);
    paths.extend(diff_paths(git::output_bytes_in_with_index(
        repository,
        &["diff", "--name-status", "-z", "--no-renames", "--"],
        index_file,
    )?)?);
    paths.extend(nul_paths(git::output_bytes_in_with_index(
        repository,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        index_file,
    )?)?);
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn reject_hidden_index_paths(repository: &Path, index_file: Option<&OsStr>) -> Result<(), String> {
    let entries =
        git::output_bytes_in_with_index(repository, &["ls-files", "-v", "-z"], index_file)?;
    let hidden = entries
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let (&tag, path) = entry.split_first()?;
            let path = path.strip_prefix(b" ")?;
            (tag == b'S' || tag.is_ascii_lowercase()).then_some(path)
        })
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map_err(|_| "Git returned a non-UTF-8 hidden index path".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if hidden.is_empty() {
        return Ok(());
    }

    Err(format!(
        "Slice scope cannot observe paths marked assume-unchanged or skip-worktree:\n{}",
        hidden
            .iter()
            .map(|path| format!("- {path}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn diff_paths(bytes: Vec<u8>) -> Result<Vec<String>, String> {
    let mut fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut paths = Vec::new();
    while let Some(status) = fields.next() {
        if status.len() != 1 || !status[0].is_ascii_uppercase() {
            return Err("Git returned an invalid name-status record".to_owned());
        }
        let path = fields
            .next()
            .ok_or_else(|| "Git returned a name-status record without a path".to_owned())?;
        paths.push(
            String::from_utf8(path.to_vec())
                .map_err(|_| "Git returned a non-UTF-8 changed path".to_owned())?,
        );
    }
    Ok(paths)
}

fn nul_paths(bytes: Vec<u8>) -> Result<Vec<String>, String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map_err(|_| "Git returned a non-UTF-8 changed path".to_owned())
        })
        .collect()
}

fn parse_rules(raw: &[String]) -> Result<Vec<PathRule>, String> {
    raw.iter().map(|value| PathRule::parse(value)).collect()
}

impl PathRule {
    fn parse(value: &str) -> Result<Self, String> {
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

    fn matches(&self, path: &str) -> bool {
        match self {
            Self::Exact(exact) => path == exact,
            Self::Tree(tree) => path == tree || path.starts_with(&format!("{tree}/")),
        }
    }

    fn contains(&self, other: &Self) -> bool {
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

    fn display(&self) -> String {
        match self {
            Self::Exact(path) => path.clone(),
            Self::Tree(path) => format!("{path}/**"),
        }
    }
}

fn overlaps(left: &[PathRule], right: &[PathRule]) -> Vec<(PathRule, PathRule)> {
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

fn json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| format!("cannot encode check result: {error}"))
}

#[cfg(test)]
mod tests;
