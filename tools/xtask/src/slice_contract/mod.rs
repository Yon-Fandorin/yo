mod binding;
mod model;
mod parallel;
mod scope;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

pub(crate) use binding::{
    bind, binding_path_for, bound_slice, ensure_bound, trusted_bound_slice, verify_bound_exact,
};
pub(crate) use model::BoundSlice;
// Preserve the prior crate-visible facade type path; current callers infer it.
#[allow(unused_imports)]
pub(crate) use model::EnsuredBinding;
#[cfg(test)]
use model::PathRule;
pub(crate) use parallel::check_parallel;
#[cfg(test)]
use parallel::overlaps;
pub(crate) use scope::{check_bound_scope, check_scope, trusted_check_bound_scope};
#[cfg(test)]
use scope::{check_bound_scope_with_index, check_scope_with_index};
use serde::Serialize;

use crate::git;

fn repository_root(directory: &Path) -> Result<PathBuf, String> {
    repository_root_with(directory, false)
}

fn repository_root_with(directory: &Path, trusted: bool) -> Result<PathBuf, String> {
    let arguments = ["rev-parse", "--show-toplevel"];
    let output = git_output(directory, &arguments, trusted)?;
    let root = output.trim();
    if root.is_empty() {
        return Err("git rev-parse --show-toplevel returned an empty path".to_owned());
    }
    Ok(PathBuf::from(root))
}

fn git_output(repository: &Path, arguments: &[&str], trusted: bool) -> Result<String, String> {
    if trusted {
        git::trusted_output_in(repository, arguments)
    } else {
        git::output_in(repository, arguments, false)
    }
}

fn git_succeeds(repository: &Path, arguments: &[&str], trusted: bool) -> Result<bool, String> {
    if trusted {
        git::trusted_succeeds_in(repository, arguments)
    } else {
        git::succeeds_in(repository, arguments, false)
    }
}

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| format!("cannot encode check result: {error}"))
}
