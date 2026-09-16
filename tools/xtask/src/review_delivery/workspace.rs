use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::Command,
};

use super::process::exit_label;
use crate::{git, slice_contract};

pub(super) fn output_directory(repository: &Path, requested: &str) -> Result<PathBuf, String> {
    let (root, coordination) = coordination_directory(repository)?;
    let requested = requested_output_path(&root, requested);
    require_real_directory(&requested)?;
    let requested = fs::canonicalize(&requested).map_err(|error| {
        format!(
            "cannot resolve review delivery output directory {}: {error}",
            requested.display()
        )
    })?;
    require_output_child(&coordination, &requested)?;
    Ok(requested)
}

fn prepare_output_directory(repository: &Path, requested: &str) -> Result<PathBuf, String> {
    let (root, coordination) = coordination_directory(repository)?;
    let requested = requested_output_path(&root, requested);
    prepare_output_directory_at(&coordination, &requested)
}

pub(super) fn delivery_output_directory(
    repository: &Path,
    requested: &str,
    prepare_output: bool,
) -> Result<PathBuf, String> {
    if prepare_output {
        prepare_output_directory(repository, requested)
    } else {
        let output = output_directory(repository, requested)?;
        require_empty_directory(&output)?;
        Ok(output)
    }
}

fn coordination_directory(repository: &Path) -> Result<(PathBuf, PathBuf), String> {
    let bound = slice_contract::trusted_bound_slice(repository)?;
    let root = common_workspace_root(repository)?;
    let coordination = root
        .join(".local-exclude")
        .join("coordination")
        .join(bound.slice);
    let coordination = fs::canonicalize(&coordination).map_err(|error| {
        format!(
            "cannot resolve Slice coordination directory {}: {error}",
            coordination.display()
        )
    })?;
    Ok((root, coordination))
}

fn requested_output_path(root: &Path, requested: &str) -> PathBuf {
    let requested = PathBuf::from(requested);
    if requested.is_absolute() {
        requested
    } else {
        root.join(requested)
    }
}

pub(super) fn prepare_output_directory_at(
    coordination: &Path,
    requested: &Path,
) -> Result<PathBuf, String> {
    let coordination = fs::canonicalize(coordination).map_err(|error| {
        format!(
            "cannot resolve Slice coordination directory {}: {error}",
            coordination.display()
        )
    })?;
    match fs::symlink_metadata(requested) {
        Ok(_) => require_real_directory(requested)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = requested.parent().ok_or_else(|| {
                "review delivery output directory must have an existing parent".to_owned()
            })?;
            let name = requested.file_name().ok_or_else(|| {
                "review delivery output directory must end in a directory name".to_owned()
            })?;
            let parent = fs::canonicalize(parent).map_err(|error| {
                format!(
                    "cannot resolve review delivery output parent {}: {error}",
                    parent.display()
                )
            })?;
            if parent != coordination && !parent.starts_with(&coordination) {
                return Err(format!(
                    "review delivery output directory must be a child of {}",
                    coordination.display()
                ));
            }
            let created = parent.join(name);
            fs::create_dir(&created).map_err(|error| {
                format!(
                    "cannot create review delivery output directory {}: {error}",
                    created.display()
                )
            })?;
        },
        Err(error) => {
            return Err(format!(
                "cannot inspect output directory {}: {error}",
                requested.display()
            ));
        },
    }
    require_real_directory(requested)?;
    let requested = fs::canonicalize(requested).map_err(|error| {
        format!(
            "cannot resolve review delivery output directory {}: {error}",
            requested.display()
        )
    })?;
    require_output_child(&coordination, &requested)?;
    require_empty_directory(&requested)?;
    verify_directory_writable(&requested)?;
    require_empty_directory(&requested)?;
    Ok(requested)
}

fn require_output_child(coordination: &Path, requested: &Path) -> Result<(), String> {
    if requested == coordination || !requested.starts_with(coordination) {
        return Err(format!(
            "review delivery output directory must be a child of {}",
            coordination.display()
        ));
    }
    Ok(())
}

fn require_real_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect output directory {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err("review delivery output must be a real directory".to_owned());
    }
    Ok(())
}

fn verify_directory_writable(path: &Path) -> Result<(), String> {
    let probe = path.join(".yo-review-delivery-write-probe");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|error| {
            format!(
                "review delivery output directory {} is not writable: {error}",
                path.display()
            )
        })?;
    drop(file);
    fs::remove_file(&probe).map_err(|error| {
        format!(
            "cannot remove review delivery output write probe {}: {error}",
            probe.display()
        )
    })
}

pub(super) fn shared_path(repository: &Path, requested: &str) -> Result<PathBuf, String> {
    let requested = PathBuf::from(requested);
    if requested.is_absolute() {
        Ok(requested)
    } else {
        common_workspace_root(repository).map(|root| root.join(requested))
    }
}

fn common_workspace_root(repository: &Path) -> Result<PathBuf, String> {
    let common = git::trusted_output_in(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(common.trim());
    if common.file_name().and_then(|name| name.to_str()) != Some(".git") {
        return Err("trusted Git common directory is not the repository .git directory".to_owned());
    }
    common
        .parent()
        .map(Path::to_owned)
        .ok_or_else(|| "trusted Git common directory has no workspace parent".to_owned())
}

pub(super) fn require_empty_directory(path: &Path) -> Result<(), String> {
    require_real_directory(path)?;
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("cannot read output directory {}: {error}", path.display()))?;
    if entries
        .next()
        .transpose()
        .map_err(|error| {
            format!(
                "cannot inspect output directory {}: {error}",
                path.display()
            )
        })?
        .is_some()
    {
        return Err(
            "review delivery output directory must be empty before its one attempt".to_owned(),
        );
    }
    Ok(())
}

pub(super) fn integration_worktree(
    repository: &Path,
    expected_commit: &str,
) -> Result<PathBuf, String> {
    let output = git::trusted_output_in(repository, &["worktree", "list", "--porcelain"])?;
    let branch = "refs/heads/develop";
    let mut matches = output
        .split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut observed_branch = None;
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("worktree ") {
                    path = Some(PathBuf::from(value));
                } else if let Some(value) = line.strip_prefix("branch ") {
                    observed_branch = Some(value);
                }
            }
            (observed_branch == Some(branch)).then_some(path).flatten()
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "expected exactly one checked-out develop integration worktree, found {}",
            matches.len()
        ));
    }
    let integration = fs::canonicalize(matches.remove(0))
        .map_err(|error| format!("cannot resolve develop integration worktree: {error}"))?;
    require_integration_state(&integration, expected_commit)?;
    Ok(integration)
}

pub(super) fn require_integration_state(
    integration: &Path,
    expected_commit: &str,
) -> Result<(), String> {
    let head = git::trusted_output_in(integration, &["rev-parse", "HEAD"])?;
    if head.trim() != expected_commit {
        return Err(format!(
            "develop integration worktree changed: expected {expected_commit}, found {}",
            head.trim()
        ));
    }
    let status = git::trusted_output_in(
        integration,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    if !status.trim().is_empty() {
        return Err("develop integration worktree must be clean before review delivery".to_owned());
    }
    Ok(())
}

pub(super) fn build_current_yo(integration: &Path) -> Result<PathBuf, String> {
    let status = Command::new("cargo")
        .args([
            "build", "--quiet", "--locked", "-p", "yo-cli", "--bin", "yo",
        ])
        .env_remove("CARGO_TARGET_DIR")
        .current_dir(integration)
        .status()
        .map_err(|error| format!("cannot start current-develop yo build: {error}"))?;
    if !status.success() {
        return Err(format!(
            "current-develop yo build failed ({}) before any delivery claim",
            exit_label(&status)
        ));
    }
    let binary = integration.join("target").join("debug").join("yo");
    let metadata = fs::symlink_metadata(&binary).map_err(|error| {
        format!(
            "cannot inspect current-develop yo binary {}: {error}",
            binary.display()
        )
    })?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("current-develop yo build did not produce a regular binary".to_owned());
    }
    Ok(binary)
}
